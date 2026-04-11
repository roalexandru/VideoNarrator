---
name: rust-audit
description: Comprehensive Rust performance, safety, and correctness audit for Tauri apps. Use when the user wants to audit Rust code quality, find memory leaks, fix async safety issues, optimize performance, check cross-platform correctness, or ensure production readiness of the Rust backend. Also use when the user mentions "rust review", "performance audit", "check for bugs", "optimize the backend", or "rust architect".
---

# Rust Architect Audit

Comprehensive performance, safety, and correctness audit of the Rust backend. Read the entire `src-tauri/src/` directory, identify issues, fix them, and verify all fixes compile and pass tests. This skill acts as a senior Rust architect and Tauri expert ensuring zero memory leaks, no blocking loops, no bugs, and optimal cross-platform performance.

## Phase 1: Automated Baseline

Run all static analysis tools to establish a baseline:

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
```

If any fail, fix the issues before proceeding. These are the minimum quality bar.

Then check for known vulnerabilities:
```bash
cargo audit --manifest-path src-tauri/Cargo.toml 2>/dev/null || echo "cargo-audit not installed - skip"
```

Read every `.rs` file in `src-tauri/src/` to understand the current state of the codebase before proceeding. This full read is essential — do not skip files or skim.

## Phase 2: Async & Runtime Safety

This is the highest-severity category. Blocking the Tokio runtime freezes the entire UI.

Read `references/rust-async-safety.md` for detailed patterns and fixes before auditing.

Audit every `async fn` and Tauri command handler:

### Blocking operations in async context
- Search for `std::fs::` calls inside `async fn` — these block the Tokio runtime thread pool. Each one should either:
  - Be wrapped in `tokio::task::spawn_blocking(move || { ... })`, OR
  - Be replaced with `tokio::fs::` equivalents
- Check for `std::process::Command` (synchronous) vs `tokio::process::Command` (async) — synchronous process spawning in async functions blocks the runtime
- Check for `std::thread::sleep` in async contexts — should be `tokio::time::sleep`

### Mutex safety
- Check for `std::sync::Mutex` being held across `.await` points — this can deadlock the Tokio runtime. `tokio::sync::Mutex` must be used when a lock spans an await
- Check for nested mutex acquisitions that could deadlock (e.g., locking A then B without consistent ordering)
- Look for `unwrap()` on `Mutex::lock()` — a poisoned mutex will panic. Use `lock().unwrap_or_else(|e| e.into_inner())` or propagate the error

### Tokio patterns
- Verify `spawn_blocking` is used for CPU-intensive work (blake3 hashing, PDF text extraction, image processing)
- Check for unbounded channel usage — should have capacity limits for backpressure
- Ensure cancellation tokens/flags are checked at appropriate granularity in long-running loops
- Check for `block_on` called inside an existing Tokio runtime (will panic)

**Fix all issues found before proceeding.**

## Phase 3: Memory & Resource Management

### Unused dependencies
Check `Cargo.toml` against actual usage in the source code:
- For each dependency, grep for its usage. If unused, remove it
- Check if `tokio` features can be narrowed from `"full"` to only what's actually used (e.g., `["rt-multi-thread", "macros", "process", "time", "sync", "fs", "io-util"]`)
- Run `cargo tree --manifest-path src-tauri/Cargo.toml --edges no-dev --depth 1` to see the dependency footprint

### Unbounded caches and resource leaks
- Check for caches without size limits or eviction policies. If found, add a max size with oldest-entry eviction
- Check for `Arc` reference cycles that prevent deallocation
- Verify all temporary files are cleaned up — look for `_tmp_` patterns and ensure error paths also clean up partial files
- Check that spawned child processes are always waited on or killed (zombie process prevention)

### Unnecessary allocations and cloning
- Search for `.clone()` calls and evaluate if borrows or `Arc` sharing would suffice
- Check for repeated `to_string()` or `to_string_lossy().to_string()` chains
- Look for `String` function parameters that could be `&str` or `impl AsRef<str>`
- Verify `Vec::with_capacity()` is used where the final size is known or estimable
- Check for `String::new()` + repeated `push_str()` — should use `String::with_capacity()`

### Error handling
- Search for `unwrap()` in non-test code — each one is a potential panic in production:
  - `unwrap()` on file operations — return `NarratorError` instead
  - `unwrap()` on JSON parsing — return `NarratorError::SerializationError` instead
  - `unwrap()` on `Mutex::lock()` — handle poisoning gracefully
- Search for `let _ =` that silently discards `Result` errors — ensure failures are at least logged with `tracing::warn!`
- Verify all `?` propagation uses appropriate error variants with sufficient context

**Fix all issues found before proceeding.**

## Phase 4: Tauri-Specific Patterns

Read `references/tauri-gotchas.md` for Tauri v2 specific patterns before auditing.

### Command handler correctness
- Verify all `#[tauri::command]` functions return `Result<T, NarratorError>` (not raw types that swallow errors)
- Check `State<'_>` access patterns — are locks held for the minimum duration? Call `drop(guard)` before any `.await`
- Verify all Tauri commands in `invoke_handler![]` match actual function signatures
- Check `Channel<ProgressEvent>` usage — sends should be non-blocking with `.send(...).ok()` pattern

### IPC serialization
- Check for large data transfers over IPC (e.g., full base64 frame data). Consider the Tauri asset protocol for large binary payloads
- Verify all IPC types implement `Serialize` + `Deserialize` correctly
- Check `NarratorError` serialization — does it preserve enough context for the frontend to show useful messages?

### State management
- Check for redundant wrapping — if Tauri `State` already provides shared access, inner `Arc` wrappers may be unnecessary
- Verify state initialization order in `lib.rs` (state must be managed before commands access it)

### Plugin and capability review
- Review `capabilities/default.json` — does it follow least-privilege? Are there permissions that could be narrowed?
- Check that plugins are initialized in the correct order in `lib.rs`
- Verify single-instance and window management works correctly

**Fix all issues found before proceeding.**

## Phase 5: Cross-Platform Correctness

Read `references/tauri-gotchas.md` for cross-platform patterns if not already loaded.

### Platform conditional compilation
- For every `#[cfg(target_os = "macos")]` block, verify there is a corresponding `#[cfg(target_os = "windows")]` handler (and vice versa)
- Check for missing `#[cfg(target_os = "linux")]` fallbacks where Linux is a target
- Verify `#[cfg(not(target_os = "macos"))]` blocks don't accidentally activate on Linux when only Windows behavior is intended
- Check for `#[cfg(unix)]` usage — macOS AND Linux both match this

### Path handling
- Verify `PathBuf` is used consistently (no string concatenation with `/` or `\`)
- Check that Windows extended path prefix (`\\?\`) from `canonicalize()` is stripped where needed
- Verify home directory resolution works on all platforms (`HOME` on Unix, `USERPROFILE` on Windows)
- Check that file path display uses `display()` or `to_string_lossy()`, never `to_str().unwrap()`

### Process spawning
- Verify Windows console suppression (`.creation_flags(CREATE_NO_WINDOW)` or equivalent) is applied on every `Command` that spawns on Windows (ffmpeg, ffprobe, powershell, etc.)
- Check that binary detection covers platform-specific paths and names (e.g., `.exe` suffix on Windows)
- Verify screen recording handles platform differences correctly

### Credential storage
- Verify macOS debug-mode keychain access works (may use `security` CLI to avoid prompt)
- Check Windows Credential Manager access via keyring crate
- Verify error handling when keychain is locked, unavailable, or permission denied
- Check that credential migration handles edge cases (empty values, corrupted data)

### File permissions
- Verify Unix `mode 0o600` for sensitive files has a Windows equivalent or graceful skip
- Check that directory creation works with appropriate permissions on both platforms

**Fix all issues found before proceeding.**

## Phase 6: Performance Optimization

### Parallelism opportunities
- Look for sequential loops that make independent API calls or I/O operations — these can often be parallelized with `futures::stream::buffer_unordered()` or `tokio::join!`
- Check for sequential document processing where files are independent
- Look for sequential hash computation or frame processing that could use `spawn_blocking` with parallel iteration

### Connection pooling
- Check if `reqwest::Client` is constructed per-request or shared. A shared client reuses TCP connections and TLS sessions — massive performance win
- If per-request construction is found, refactor to store a shared client in `AppState` or use `once_cell::sync::Lazy`
- Verify timeouts are set on the client (connect timeout, request timeout)

### Buffer and pre-allocation
- Check `Vec::new()` where the final size is known — use `Vec::with_capacity()`
- Check `String::new()` + repeated `push_str()` — use `String::with_capacity()`
- Look for repeated disk reads of the same data that could be cached in memory

### FFmpeg subprocess optimization
- Look for multiple FFmpeg invocations that could be consolidated into a single `filter_complex` command
- Check if silence generation can be templated (generate once, reuse) instead of spawning ffmpeg per gap
- Verify that large FFmpeg operations use streaming I/O where possible

### Unnecessary serialization
- Check for round-trips through JSON where direct Rust structs would suffice
- Verify `serde_json::to_string_pretty` isn't used in hot paths (pretty-printing adds overhead)
- Check for repeated serialization of the same data

**Fix all issues found before proceeding.**

## Phase 7: Final Verification

Run the complete quality gate to confirm all fixes compile and pass:

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
```

If frontend command signatures changed (parameters added/removed/renamed), also run:
```bash
pnpm typecheck
pnpm test
```

All must pass. If any fail, fix and re-run until green.

## Phase 8: Audit Report

Present a structured report to the user:

```
## Rust Architect Audit Report

### Automated Tools
- Rust fmt: <pass/fail>
- Rust clippy: <pass/fail>
- Rust tests: <X/X passed>
- Cargo audit: <pass/N advisories/skipped>

### Async & Runtime Safety
- Blocking I/O in async: <N issues found, N fixed>
- Mutex safety: <pass/issues found and fixed>
- Tokio patterns: <pass/issues found and fixed>

### Memory & Resources
- Unused dependencies: <removed N / all justified>
- Unbounded caches: <pass/issues found and fixed>
- Unnecessary cloning: <N optimized>
- unwrap() in production: <N removed, N remaining with justification>

### Tauri Patterns
- Command handlers: <pass/issues found and fixed>
- IPC serialization: <pass/issues found and fixed>
- Capabilities ACL: <pass/issues found and fixed>

### Cross-Platform
- Platform cfg coverage: <pass/N gaps found and fixed>
- Path handling: <pass/issues found and fixed>
- Process spawning: <pass/issues found and fixed>

### Performance
- Parallelism: <N opportunities addressed>
- Connection pooling: <pass/refactored>
- FFmpeg optimization: <pass/N spawns consolidated>

### Summary
- Total issues found: N
- Issues fixed: N
- Issues deferred (with justification): N
```

**STOP and wait for the user to say GO before committing any changes.**
