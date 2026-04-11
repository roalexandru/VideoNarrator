# Tauri v2 Cross-Platform Gotchas & Patterns

Reference guide for Tauri-specific issues in cross-platform desktop apps. Read this before auditing Phases 4 and 5.

## IPC (Inter-Process Communication)

### Serialization Overhead

Every Tauri command call serializes arguments to JSON, sends them across the IPC bridge, deserializes in Rust, then serializes the return value back. For large payloads this is a bottleneck.

**Red flags:**
- Sending base64-encoded images or video frames over IPC (each frame could be 100KB+)
- Returning large `Vec<u8>` from commands
- Sending entire file contents as strings

**Fixes:**
- Use the Tauri asset protocol (`asset://`) for large binary data — the frontend reads directly from disk
- For frame thumbnails: write to disk, return the file path, let the frontend load via asset protocol
- For streaming data: use `Channel<T>` with small incremental payloads, not one large batch
- Consider pagination for list commands that could return hundreds of items

### Error Serialization

Tauri commands that return `Result<T, E>` serialize the error as a string via `Display`. If your error type has structured data (error codes, context), it's lost.

**Pattern:** Implement `serde::Serialize` on your error type and return it as a structured JSON error:
```rust
#[derive(Debug, thiserror::Error, serde::Serialize)]
pub enum NarratorError {
    #[error("FFmpeg not found")]
    FfmpegNotFound,
    // ...
}

// Tauri requires this impl:
impl serde::Serialize for NarratorError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where S: serde::Serializer {
        serializer.serialize_str(&self.to_string())
    }
}
```

### Channel<T> for Progress

`Channel<T>` is Tauri's built-in streaming mechanism from Rust to frontend. Key patterns:
- `channel.send(event).ok()` — never `unwrap()`, the frontend may have navigated away
- Sends are non-blocking — they queue the message for the IPC bridge
- Don't send too frequently (>60 events/sec) — it floods the IPC bridge. Throttle to ~10-20 events/sec for progress bars
- The channel is automatically cleaned up when the command returns

## State Management

### Tauri State<'_> Patterns

`tauri::State<'_, T>` provides shared immutable access to managed state. Interior mutability requires `Mutex`, `RwLock`, or atomics.

**Common mistake — redundant Arc:**
```rust
// WRONG: Tauri State already wraps in Arc internally
pub struct AppState {
    pub data: Arc<Mutex<HashMap<String, String>>>,
}

// RIGHT: Just use Mutex directly
pub struct AppState {
    pub data: Mutex<HashMap<String, String>>,
}
```

Exception: If you need to pass a reference to the data into a spawned task (`tokio::spawn`), you DO need `Arc` because the spawned task must own its references:
```rust
pub struct AppState {
    pub cancel_flag: Arc<AtomicBool>, // Shared with spawned tasks - Arc justified
    pub config: Mutex<Config>,         // Only accessed in command handlers - no Arc needed
}
```

### Lock Duration

In Tauri commands, lock the state, extract what you need, drop the lock, then do async work:

```rust
#[tauri::command]
async fn my_command(state: State<'_, AppState>) -> Result<String, NarratorError> {
    // GOOD: Short lock scope
    let api_key = {
        let keys = state.api_keys.lock().await;
        keys.get("openai").cloned().ok_or(NarratorError::NoApiKey)?
    }; // Lock dropped here

    // Now do async work without holding the lock
    let result = call_api(&api_key).await?;
    Ok(result)
}
```

**Red flag:** Any `.await` while a `State` lock guard is alive.

### State Initialization Order

In `lib.rs`, `.manage(state)` must be called before any command can access it. If a plugin or setup hook accesses state, ensure it's managed before the plugin is added:

```rust
tauri::Builder::default()
    .manage(app_state)      // Must come before plugins that access it
    .plugin(my_plugin)
    .invoke_handler(...)
```

## Cross-Platform Path Handling

### Windows Extended Path Prefix

`std::fs::canonicalize()` on Windows returns paths prefixed with `\\?\` (extended-length path prefix). This prefix:
- Breaks string comparison with paths from other sources
- Confuses some subprocess tools (including some FFmpeg builds)
- Displays ugly paths to users

**Fix:** Strip the prefix after canonicalization:
```rust
fn clean_path(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if s.starts_with(r"\\?\") {
        PathBuf::from(&s[4..])
    } else {
        path.to_path_buf()
    }
}
```

Apply this everywhere `canonicalize()` is called, or avoid `canonicalize()` when possible (use `std::path::absolute()` on Rust 1.79+ or `dunce::canonicalize()` which strips the prefix automatically).

### Path Separators

Never concatenate paths with `/` or `\`. Always use `PathBuf::join()`:
```rust
// WRONG
let path = format!("{}/frames/{}.jpg", project_dir, frame_id);

// RIGHT
let path = project_dir.join("frames").join(format!("{}.jpg", frame_id));
```

### Home Directory

`dirs::home_dir()` or the `directories` crate handles cross-platform home resolution:
- macOS/Linux: `$HOME` (`/Users/name` or `/home/name`)
- Windows: `USERPROFILE` (`C:\Users\name`)

Never hardcode `~` in paths — it's not expanded by Rust's `std::fs`. Always resolve to absolute paths.

### Temp Directories

Use `std::env::temp_dir()` — it returns the correct platform-specific temp directory:
- macOS: `/var/folders/.../T/` (per-user temp)
- Windows: `C:\Users\name\AppData\Local\Temp\`
- Linux: `/tmp`

## Process Spawning

### Windows Console Window

On Windows, spawning a `Command` (ffmpeg, ffprobe, etc.) opens a visible console window by default. Suppress it:

```rust
use std::process::Command;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

let mut cmd = Command::new("ffmpeg");
#[cfg(target_os = "windows")]
cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW

cmd.args(&["-i", "input.mp4"]);
```

The constant `0x08000000` is `CREATE_NO_WINDOW`. Apply this to EVERY process spawned on Windows, including PowerShell and helper utilities.

### Binary Resolution

- macOS: Check `PATH`, `/usr/local/bin/`, Homebrew paths, bundled sidecar
- Windows: Check `PATH`, bundled sidecar, common install locations. Binary names need `.exe` suffix
- Linux: Check `PATH`, `/usr/bin/`, snap/flatpak paths

For bundled sidecars, Tauri provides `app.path().resource_dir()` to find them.

### Process Cleanup

Always ensure child processes are cleaned up:
```rust
let mut child = Command::new("ffmpeg").spawn()?;

// If we need to cancel:
child.kill().ok();  // Send kill signal
child.wait().ok();  // Wait for process to actually exit (prevents zombies)
```

On Windows, `kill()` terminates immediately. On Unix, it sends SIGKILL. For graceful shutdown, consider sending input to stdin first (e.g., `q\n` for FFmpeg), waiting with a timeout, then killing.

## File Permissions

### Unix vs Windows

`std::fs::set_permissions()` with Unix mode bits (e.g., `0o600`) has no effect on Windows. Windows uses ACLs.

**Pattern:**
```rust
#[cfg(unix)]
{
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
}
// On Windows, file permissions are inherited from the parent directory
// and controlled by ACLs. For user-private files, storing in AppData is sufficient.
```

### Atomic File Writes

To prevent corruption from crashes mid-write:
```rust
let tmp_path = path.with_extension("tmp");
std::fs::write(&tmp_path, &data)?;
std::fs::rename(&tmp_path, &path)?; // Atomic on most filesystems
```

This works on both macOS and Windows (same-volume rename is atomic).

## Credential Storage

### Platform Differences

| Platform | Backend | Notes |
|----------|---------|-------|
| macOS | Keychain (Security.framework) | May prompt for permission on first access. Debug builds may prompt repeatedly |
| Windows | Credential Manager (wincred) | 2KB limit per credential. No prompts |
| Linux | Secret Service (libsecret/gnome-keyring) | Requires running secret service daemon |

### macOS Debug Keychain Prompt

In debug builds, the keychain prompts for access because the binary isn't signed. Workaround: use the `security` CLI to access keychain entries, which may avoid some prompts:
```rust
#[cfg(all(target_os = "macos", debug_assertions))]
fn read_keychain(service: &str, account: &str) -> Option<String> {
    let output = std::process::Command::new("security")
        .args(["find-generic-password", "-s", service, "-a", account, "-w"])
        .output().ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}
```

### Windows Credential Size Limit

Windows Credential Manager has a ~2.5KB limit per credential. For multiple API keys, store them as a single JSON blob rather than individual entries:
```json
{"openai": "sk-...", "anthropic": "sk-ant-...", "elevenlabs": "..."}
```

This also reduces the number of keychain operations from N to 1.

## Tauri Updater

### Asset Naming

The Tauri auto-updater on Windows references assets by their exact filename in `latest.json`. Renaming Windows installer assets (`.exe`, `.msi`, `.nsis.zip`) after upload breaks auto-update. Only rename macOS DMGs (which aren't referenced in the update manifest).

### Signature Files

Every updatable asset needs a corresponding `.sig` file. The Tauri build process generates these when `TAURI_SIGNING_PRIVATE_KEY` is set. Without signatures, the updater rejects the update.

## Webview Differences

### Process Model

- **macOS**: WebKit runs in-process (single process)
- **Windows**: WebView2 (Chromium-based) runs out-of-process (multi-process model)
- **Linux**: WebKitGTK runs in-process

This means Windows has higher base memory usage but better crash isolation.

### CSP (Content Security Policy)

Tauri v2 enforces CSP in `tauri.conf.json`. Common gotchas:
- `connect-src` must list all API endpoints the frontend calls directly (not Rust-proxied calls)
- `img-src` must include `asset:` and `https://asset.localhost` for the asset protocol
- Inline styles are allowed by default in Tauri v2, but inline scripts are not
- `unsafe-eval` should be avoided — it opens XSS attack surface

### Navigation

Tauri v2 blocks external navigation by default. If a link needs to open in the system browser, use `tauri-plugin-opener` instead of `window.open()`.

## Plugin Lifecycle

### Initialization Order

Plugins initialize in the order they're added to the builder. Dependencies between plugins must be ordered correctly:
```rust
tauri::Builder::default()
    .plugin(tauri_plugin_fs::init())       // Must come before plugins that use fs
    .plugin(tauri_plugin_dialog::init())
    .plugin(tauri_plugin_updater::init())  // May depend on fs for checking updates
```

### Setup Hook Timing

`Builder::setup()` runs after all plugins are initialized but before the first window is created. State must be managed before `setup()` if the setup hook accesses it.

## Performance Anti-Patterns

### Frequent Small IPC Calls

Each IPC call has overhead (~1-5ms for serialization + bridge crossing). Batching multiple small reads into one command is significantly faster:

```rust
// SLOW: N IPC calls
for id in &ids {
    let item = invoke("get_item", id).await;
}

// FAST: 1 IPC call
let items = invoke("get_items_batch", &ids).await;
```

### Large State Under Single Lock

If `AppState` has one large `Mutex`, all commands contend on it even when accessing independent fields. Split into separate locks:

```rust
// SLOW: Single lock for everything
pub struct AppState {
    pub everything: Mutex<AllTheThings>,
}

// FAST: Independent locks for independent data
pub struct AppState {
    pub api_keys: Mutex<HashMap<String, String>>,
    pub cancel_flag: Arc<AtomicBool>,
    pub config: Mutex<Config>,
}
```
