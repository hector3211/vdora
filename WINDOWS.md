# Vdora for Windows (x64)

Vdora runs from a terminal and transcribes locally. This build does not include the Linux GUI, tray icon, or auto-paste.

## Run the portable build

1. Download `vdora-windows-x86_64.zip` from [GitHub Releases](https://github.com/hector3211/vdora/releases) and extract it. If Windows reports a missing `VCRUNTIME` or `MSVCP` DLL, install the [Microsoft Visual C++ x64 Redistributable](https://aka.ms/vs/17/release/vc_redist.x64.exe).
2. Open PowerShell in the extracted folder and run `.\vdora.exe`. The first run downloads the default English Whisper model (about 148 MB). Once the model is on disk, dictation works offline.
3. Leave Vdora running. From another app, press **Ctrl+Alt+Space** to start recording, then press it again to stop. Vdora stops automatically after two minutes. Wait for **Transcript copied**, then press **Ctrl+V** in your app.
4. To quit, return to PowerShell and press **Ctrl+C**.

If the microphone does not work, check the default input in **Settings > System > Sound** and allow desktop apps to access your microphone in **Settings > Privacy & security > Microphone**. If Vdora cannot register the shortcut, close any other instance of Vdora or the app that already uses it.

The default model is stored under `%LOCALAPPDATA%\vdora\vdora\data\models`. To use a different model, create `%APPDATA%\vdora\vdora\config\config.toml` and set `model_path` to its full path (use single quotes so TOML treats Windows backslashes literally). For a multilingual model, you can set `language = "fr"`, for example, or omit it for automatic detection.

## Build from source

Use an x64 Developer PowerShell for Visual Studio with **Desktop development with C++** and the Windows SDK installed. Install Rust 1.92 or newer for `x86_64-pc-windows-msvc`, CMake, and LLVM (set `LIBCLANG_PATH` to the directory containing `libclang.dll` if needed). From the repository root:

```powershell
$env:CXXFLAGS = '/EHsc'
cargo test --locked --release --target x86_64-pc-windows-msvc
cargo build --locked --release --target x86_64-pc-windows-msvc
.\target\x86_64-pc-windows-msvc\release\vdora.exe --help
```

To check the whole workflow on Windows, start Vdora, dictate into a different app such as Notepad, confirm the transcript pastes, and repeat without restarting Vdora. Also try quitting while recording and then relaunching; the shortcut and microphone should be available again.
