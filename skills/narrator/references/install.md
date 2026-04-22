# Installing `narrator-cli`

The skill needs `narrator-cli` somewhere on `PATH`. Three options:

## 1. From the desktop app bundle (recommended if you already have Narrator installed)

The binary ships alongside the GUI. Symlink it — prefer a user-writable location (no sudo) over `/usr/local/bin` when you can.

**macOS (no sudo, recommended):**
```bash
mkdir -p ~/.local/bin
ln -sf /Applications/Narrator.app/Contents/MacOS/narrator-cli ~/.local/bin/narrator-cli
# Make sure ~/.local/bin is on PATH (zsh/bash):
#   echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.zshrc && source ~/.zshrc
```

**macOS (system-wide, asks for sudo):**
```bash
sudo ln -sf /Applications/Narrator.app/Contents/MacOS/narrator-cli /usr/local/bin/narrator-cli
```

**Windows:**
The binary is at `C:\Program Files\Narrator\narrator-cli.exe`. Add that directory to `PATH`, or copy the .exe to a directory already on `PATH`.

**Linux (AppImage):**
Extract the AppImage (`./Narrator.AppImage --appimage-extract`) and symlink `narrator-cli` from the extracted `usr/bin/` into `~/.local/bin/`.

## 2. Build from source

From a clone of the Narrator repo:

```bash
cargo install --path src-tauri --bin narrator-cli
```

This puts the binary in `~/.cargo/bin/`, which should already be on `PATH` if you have a Rust toolchain installed.

## 3. Run directly from a build

```bash
cd path/to/VideoNarator
cargo build --release --manifest-path src-tauri/Cargo.toml --bin narrator-cli
# then point to src-tauri/target/release/narrator-cli directly
```

## Verify the install

```bash
narrator-cli --version
narrator-cli probe video --input some-test-video.mp4
```

The first command prints the version string. The second should print a JSON envelope with the file's metadata (or an error envelope if ffmpeg isn't installed).

## ffmpeg requirement

`narrator-cli` shells out to `ffmpeg` and `ffprobe`. Both must be on `PATH`:

- **macOS:** `brew install ffmpeg`
- **Windows:** [official builds](https://www.gyan.dev/ffmpeg/builds/), add `bin/` to `PATH`.
- **Linux:** `sudo apt install ffmpeg` / `sudo dnf install ffmpeg`.
