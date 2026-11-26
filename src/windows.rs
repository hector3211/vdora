use std::{io, ptr::null_mut};

use anyhow::{Context, Result, bail};
use windows_sys::Win32::UI::{
    Input::KeyboardAndMouse::{
        MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey, VK_SPACE,
    },
    WindowsAndMessaging::{
        GetMessageW, KillTimer, MSG, PM_REMOVE, PeekMessageW, SetTimer, WM_HOTKEY, WM_QUIT,
        WM_TIMER,
    },
};

use crate::{
    audio::windows::{Capture, MAX_SECONDS},
    config::AppConfig,
    init_tracing,
    stt::{model::ensure_model, whisper::WhisperService},
};

const HOTKEY_ID: i32 = 1;
const HOTKEY: &str = "Ctrl+Alt+Space";

pub fn run() -> Result<()> {
    if let Some(arg) = std::env::args_os().nth(1) {
        if arg == "--help" || arg == "-h" {
            println!(
                "Vdora\n\nUsage: vdora.exe\n\nPress {HOTKEY} to start or stop recording. Paste with Ctrl+V.\nKeep Vdora running in this terminal. Press Ctrl+C to exit."
            );
            return Ok(());
        }
        bail!(
            "unsupported argument on Windows: {}. See --help.",
            arg.to_string_lossy()
        );
    }

    let config = AppConfig::load_or_default();
    init_tracing(&config);
    let _hotkey = Hotkey::register()?;
    ensure_model(&config.model_path)?;
    eprintln!("Loading the local Whisper model...");
    let mut whisper = WhisperService::load(config.model_path.clone())?;
    if discard_pending_input() {
        return Ok(());
    }
    eprintln!("Ready. Press {HOTKEY} to start or stop recording. Press Ctrl+C to exit.");

    let mut recording: Option<(Capture, RecordingTimer)> = None;
    let mut message = MSG::default();
    loop {
        // SAFETY: message is writable, and a null HWND reads this thread's queue.
        match unsafe { GetMessageW(&mut message, null_mut(), 0, 0) } {
            -1 => return Err(io::Error::last_os_error()).context("cannot read Windows messages"),
            0 => return Ok(()),
            _ => {}
        }
        let is_hotkey = message.message == WM_HOTKEY && message.wParam == HOTKEY_ID as usize;
        let is_timeout = message.message == WM_TIMER
            && recording
                .as_ref()
                .is_some_and(|(_, timer)| message.wParam == timer.0);
        if !is_hotkey && !is_timeout {
            continue;
        }
        if let Some((capture, timer)) = recording.take() {
            drop(timer);
            if is_timeout {
                eprintln!("Reached the {MAX_SECONDS}-second recording limit.");
            }
            if let Err(err) = finish(capture, &mut whisper, config.language.as_deref()) {
                eprintln!("Vdora: {err:#}");
            }
            // Do not treat keys pressed during inference as requests to record later.
            if discard_pending_input() {
                return Ok(());
            }
            eprintln!("Ready. Press {HOTKEY} to record again.");
        } else {
            match Capture::start().and_then(|capture| Ok((capture, RecordingTimer::start()?))) {
                Ok(session) => {
                    recording = Some(session);
                    eprintln!("Recording... Press {HOTKEY} again to stop.");
                }
                Err(err) => eprintln!("Vdora: {err:#}"),
            }
        }
    }
}

fn finish(capture: Capture, whisper: &mut WhisperService, language: Option<&str>) -> Result<()> {
    let samples = capture.finish()?;
    eprintln!("Transcribing locally...");
    let transcript = whisper.transcribe_samples(&samples, language)?;
    if transcript.is_empty() {
        eprintln!("No speech detected. Clipboard unchanged.");
        return Ok(());
    }
    arboard::Clipboard::new()
        .context("cannot open the Windows clipboard")?
        .set_text(transcript)
        .context("cannot copy the transcript to the Windows clipboard")?;
    eprintln!("Transcript copied. Paste with Ctrl+V.");
    Ok(())
}

fn discard_pending_input() -> bool {
    let mut message = MSG::default();
    for kind in [WM_HOTKEY, WM_TIMER] {
        // SAFETY: message is writable and the filters select this thread's queued messages.
        while unsafe { PeekMessageW(&mut message, null_mut(), kind, kind, PM_REMOVE) } != 0 {
            // Windows can return WM_QUIT even when the range filters it out.
            if message.message == WM_QUIT {
                return true;
            }
        }
    }
    false
}

struct Hotkey;

impl Hotkey {
    fn register() -> Result<Self> {
        // SAFETY: a null HWND registers the shortcut on this thread, where it is also unregistered.
        let ok = unsafe {
            RegisterHotKey(
                null_mut(),
                HOTKEY_ID,
                MOD_CONTROL | MOD_ALT | MOD_NOREPEAT,
                u32::from(VK_SPACE),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error()).with_context(|| {
                format!("cannot register {HOTKEY}; another app or Vdora may already be using it")
            });
        }
        Ok(Self)
    }
}

impl Drop for Hotkey {
    fn drop(&mut self) {
        // SAFETY: the registration and message loop run on this same thread.
        unsafe { UnregisterHotKey(null_mut(), HOTKEY_ID) };
    }
}

struct RecordingTimer(usize);

impl RecordingTimer {
    fn start() -> Result<Self> {
        // SAFETY: a null HWND and callback request WM_TIMER messages on this thread.
        let id = unsafe { SetTimer(null_mut(), 0, (MAX_SECONDS * 1000) as u32, None) };
        if id == 0 {
            bail!("cannot start the Windows recording timer");
        }
        Ok(Self(id))
    }
}

impl Drop for RecordingTimer {
    fn drop(&mut self) {
        // SAFETY: the timer was created on the same thread with this ID.
        unsafe { KillTimer(null_mut(), self.0) };
    }
}
