# Vdora

> Speak. Transcribe locally. Paste anywhere.

Vdora is a native Linux voice-to-text app for GNOME and other GTK-based desktops. It records your voice, transcribes it **locally** with OpenAI Whisper, and copies the result to your clipboard — ready to paste into any application.

No cloud. No subscription. Your voice never leaves your machine.

## What is Vdora for?

Vdora is built for anyone who wants to talk instead of type on Linux. It works especially well as a global dictation shortcut you can trigger from anywhere.

Common uses:

- **Chat and messaging** — dictate long messages in Slack, Discord, Signal, or any chat app.
- **Writing and notes** — quickly capture thoughts in your notes app or document editor.
- **Code assistants / AI prompts** — speak a long prompt, paste it into opencode, ChatGPT, Claude, or any LLM tool.
- **Accessibility** — reduce typing when your hands are busy or when typing is uncomfortable.
- **Privacy-first transcription** — everything runs locally with Whisper; no audio is sent to the internet.

## How it works

1. Press your global shortcut (e.g. `Super+Alt+Space`) to start recording.
2. Speak.
3. Press the same shortcut again to stop recording.
4. Vdora transcribes the audio locally and copies the text to your clipboard.
5. Paste with `Ctrl+V` wherever your cursor is.

## Installation

### From GitHub Releases (recommended)

Download the latest release from [github.com/hector3211/vdora/releases](https://github.com/hector3211/vdora/releases):

- `.deb` package for Debian/Ubuntu and derivatives
- `.tar.gz` archive with a prebuilt binary for other distributions

### Build from source

Requires Rust 1.92 or newer and native build dependencies. See [Build dependencies](#build-dependencies) below.

```bash
git clone https://github.com/hector3211/vdora.git
cd vdora
cargo build --release
```

## Setup

### 1. Whisper model

Vdora needs a Whisper model. By default it expects:

```text
~/.local/share/vdora/models/ggml-base.en.bin
```

In one-shot mode, Vdora downloads a missing `ggml-base.en.bin` automatically on first use. For GUI mode, install a model first. You can also place your own model there:

```bash
mkdir -p ~/.local/share/vdora/models
cp /path/to/ggml-base.en.bin ~/.local/share/vdora/models/
```

**About S1-mini:** [Superwhisper S1-mini](https://huggingface.co/superwhisper/s1-mini) is a text-cleanup model, not an audio transcription model. It cannot be loaded as a Whisper `.bin` file. S1-mini integration would require a separate local inference stage after Whisper; that stage is not implemented yet.

### 2. Optional: enable auto-paste

By default, Vdora copies the transcript to the clipboard and you paste manually. If you install `ydotool`, Vdora can also send a paste shortcut automatically:

```bash
# Fedora
sudo dnf install ydotool

# Debian/Ubuntu
sudo apt install ydotool
```

Enable **Auto-paste** in the GUI Settings tab and choose **Paste mode**:

- **Standard** sends `Ctrl+V` for browsers, editors, and other desktop apps.
- **Terminal** sends `Ctrl+Shift+V` for WezTerm and other terminals, without changing their keybindings.

Click **Save Settings**. The selection applies to GUI paste actions and `vdora --oneshot`. Existing configurations default to Standard. Alternatively, set `paste_mode = "terminal"` (or `"standard"`) in `~/.config/vdora/config.toml`.

`ydotoold` must be running with a socket your user can access; installing `ydotool` alone is insufficient. Vdora respects `YDOTOOL_SOCKET` when set; otherwise it selects an existing socket at `$XDG_RUNTIME_DIR/.ydotool_socket`, then `/tmp/.ydotool_socket`. This handles Fedora clients and system daemons using different default locations. Do not make the socket world-writable. Automatic per-application selection is not implemented yet, so select the mode appropriate for your target app.

### 3. Recommended: global shortcut for one-shot mode

The best way to use Vdora is with a GNOME keyboard shortcut:

1. Open **Settings > Keyboard > View and Customize Shortcuts > Custom Shortcuts**.
2. Add a shortcut:
   - **Name:** Vdora
   - **Command:** `vdora --oneshot`
   - **Shortcut:** `Super+Alt+Space` (or whatever you prefer)

Now you can dictate from any app by pressing that shortcut once to start and again to stop.

## Two ways to use Vdora

### One-shot mode (recommended)

Triggered from anywhere via a global shortcut. No window needed. Perfect for quick dictation.

```bash
vdora --oneshot
```

Aliases and options:

```bash
vdora --oneshot --no-notify    # disable desktop notifications
```

### GUI mode

Run without arguments to open the full application:

```bash
vdora
```

The GUI gives you:

- A big record button with live elapsed time
- Settings (language, model path, auto-paste mode, hotkey)
- Diagnostics to verify dependencies
- System tray integration
- Native libadwaita header bar, bottom navigation on narrow windows, and adaptive settings rows
- System light/dark and high-contrast styling without a custom theme

## Example: dictating into opencode

1. Focus the opencode prompt.
2. Press your Vdora shortcut.
3. Say: *"Refactor the login function to use bcrypt and add unit tests for empty passwords."*
4. Press the shortcut again.
5. Wait for the "Transcript copied" notification.
6. Press `Ctrl+V` in opencode.

For long prompts, paste into opencode's editor with `/editor` or `Ctrl+X` then `E`, clean up the text, save, and close.

## Runtime dependencies

Required:

- `pw-record` (PipeWire tools)
- GTK 4.12+ and libadwaita 1.4+ runtime libraries

Optional:

- `ydotool` + `ydotoold` — for automatic `Ctrl+V` paste
- `wl-copy` from `wl-clipboard` — more reliable clipboard on Wayland

## Build dependencies

To build from source you need:

- Rust 1.92+ toolchain
- `cmake`, `clang`/`gcc`
- `libclang` development package (for bindgen)
- GTK4 + libadwaita development packages

Fedora example:

```bash
sudo dnf install clang clang-devel cmake gcc-c++ glib2-devel gtk4-devel libadwaita-devel pipewire-utils
```

Optional auto-paste support:

```bash
sudo dnf install ydotool
```

## Tips

- The default model (`ggml-base.en`) balances speed and accuracy well. Larger models are more accurate but slower.
- Vdora preloads and reuses the model and inference state in GUI mode, and loads them while recording in one-shot mode, to avoid repeated allocation after you stop speaking. Keeping the GUI open therefore uses additional memory roughly proportional to the selected model size. Each recording is decoded without carrying over previous dictation context.
- On Wayland, `wl-copy` is used automatically when available for better clipboard reliability.
- If auto-paste does not work, the transcript is still on your clipboard — just press `Ctrl+V`.
- If Vdora reports "No speech detected," verify the intended microphone is selected as the system's default input and try again closer to the microphone.
- Tray icon visibility on GNOME may require an AppIndicator/KStatusNotifier extension.

## License

MIT
