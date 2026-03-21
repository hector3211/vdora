# Vdora (GNOME Voice-to-Paste)

Vdora is a native GTK4/libadwaita Linux app that records your voice, transcribes locally with Whisper, copies the transcript to clipboard, and then attempts an auto-paste (`Ctrl+V`) at the current cursor target.

## Current MVP

- Record/stop button in a GNOME-native window
- Local transcription via `whisper-rs`
- Clipboard copy for every transcript
- Auto-paste via `ydotool` (with fallback if unavailable)
- Basic settings persistence (`autopaste`, `language`, `model_path`)
- No-speech recordings return to idle without raising an error
- System tray integration (show/hide window, toggle recording, quit)
- Tray tooltip reflects current app state (Idle/Recording/Transcribing/Error)
- Temporary recordings are auto-cleaned and stale `vdora-*.wav` files are swept on startup

## Install (recommended)

Use the prebuilt package from GitHub Releases when available.

- `.deb` package for Debian/Ubuntu-style systems
- prebuilt Linux binary archive (`.tar.gz`) for manual install

Building from source is still supported (see below), but end users should prefer release artifacts.

## Runtime dependencies

Required:

- `pw-record` (PipeWire tools)
- GTK4 + libadwaita runtime libraries

Optional:

- `ydotool` and `ydotoold` (only for auto-paste key injection)

## Build dependencies

The `whisper-rs` stack compiles native bindings and requires:

- Rust toolchain
- C/C++ build tools (`cmake`, `clang`, `gcc`)
- `libclang` development package (for bindgen)
- GTK4 + libadwaita development packages

Example on Fedora-like systems:

```bash
sudo dnf install clang clang-devel cmake gcc-c++ glib2-devel gtk4-devel libadwaita-devel dbus-devel pipewire-utils
```

Optional auto-paste support:

```bash
sudo dnf install ydotool
```

## Whisper model setup

By default, Vdora looks for:

`~/.local/share/vdora/models/ggml-base.en.bin`

Create the directory and place a model file there, or edit config later:

```bash
mkdir -p ~/.local/share/vdora/models
```

## Run

```bash
cargo run
```

## Notes on GNOME Wayland

- Clipboard insertion works reliably.
- Simulated paste may depend on compositor/session permissions.
- If auto-paste fails, transcript is still copied to clipboard.
- Tray icon visibility on GNOME may require an AppIndicator/KStatusNotifier extension.
- If a `vdora` icon is not installed in the icon theme, tray falls back to a microphone icon.
