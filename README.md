# Vdora (GNOME Voice-to-Paste)

Vdora is a native GTK4/libadwaita Linux app that records your voice, transcribes locally with Whisper, copies the transcript to clipboard, and then attempts an auto-paste (`Ctrl+V`) at the current cursor target.

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
- `wl-copy` from `wl-clipboard` (recommended on Wayland for reliable clipboard writes from tray/background)

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

If a model file is missing when running the default config, Vdora downloads `ggml-base.en.bin` automatically on first use. You can also create the directory and place a model file there manually:

```bash
mkdir -p ~/.local/share/vdora/models
```

## Use with opencode (recommended)

Vdora supports a one-shot CLI mode for GNOME global shortcuts. This is the most reliable way to dictate into opencode on Wayland.

### Set up the shortcut

1. Open Settings > Keyboard > View and Customize Shortcuts > Custom Shortcuts.
2. Add a shortcut with this command:

   ```bash
   vdora --oneshot --duration 30
   ```

3. Bind it to something like `Super+Alt+Space`.

### Dictate into opencode

1. Focus the opencode prompt.
2. Press the shortcut once to start recording.
3. Speak.
4. Press the same shortcut again to stop early and transcribe.
   - Or wait the full 30 seconds.
5. Paste into opencode with `Ctrl+V`.

The desktop notification updates from recording to transcribing to copied instead of stacking.

## GUI mode

Run without arguments to open the GTK app:

```bash
vdora
```

The GUI still supports the in-app hotkey when Vdora is focused, plus settings, diagnostics, and tray integration.

## opencode editor tip

For long prompts, use opencode's `/editor` command or press `Ctrl+X` then `E`, then paste the transcript into the editor, clean it up, save, and close it.

## Run

```bash
cargo run
```

## Notes on GNOME Wayland

- Clipboard insertion works reliably.
- On Wayland, Vdora prefers `wl-copy` when available (then falls back to GTK clipboard APIs).
- Simulated paste may depend on compositor/session permissions.
- If auto-paste fails, transcript is still copied to clipboard.
- Tray icon visibility on GNOME may require an AppIndicator/KStatusNotifier extension.
- If a `vdora` icon is not installed in the icon theme, tray falls back to a microphone icon.
