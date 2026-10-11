use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, anyhow};
use nix::{
    sys::signal::{Signal, kill},
    unistd::Pid,
};

use crate::{
    audio::recorder::Recorder,
    config::AppConfig,
    insert::{clipboard, paste},
    stt::whisper::WhisperService,
};

const APP_TITLE: &str = "Vdora";
const DEFAULT_MODEL_FILE: &str = "ggml-base.en.bin";
const DEFAULT_MODEL_URL: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin";

#[derive(Debug, Clone)]
pub struct Options {
    pub no_notify: bool,
}

#[derive(Debug, Clone)]
struct Paths {
    runtime_dir: PathBuf,
    state_file: PathBuf,
    lock_file: PathBuf,
    notification_id_file: PathBuf,
}

#[derive(Debug, Default)]
struct State {
    parent_pid: Option<u32>,
    recorder_pid: Option<u32>,
    phase: Option<String>,
}

pub enum Mode {
    Gui,
    Help,
    Run(Options),
}

pub fn parse_args() -> Result<Mode> {
    let mut args = std::env::args().skip(1).peekable();
    let mut oneshot = false;
    let mut no_notify = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--oneshot" | "voice" => oneshot = true,
            "--duration" | "-d" => {
                // Keep accepting existing desktop shortcuts, but recording now
                // continues until the shortcut is toggled again.
                let value = args
                    .next()
                    .ok_or_else(|| anyhow!("{arg} requires a value in seconds"))?;
                let duration = value
                    .parse::<u64>()
                    .with_context(|| format!("invalid duration: {value}"))?;
                if duration == 0 {
                    return Err(anyhow!("duration must be greater than zero"));
                }
            }
            "--no-notify" => no_notify = true,
            "--help" | "-h" => return Ok(Mode::Help),
            unknown => return Err(anyhow!("unknown argument: {unknown}")),
        }
    }

    if !oneshot {
        return Ok(Mode::Gui);
    }

    Ok(Mode::Run(Options { no_notify }))
}

pub fn print_help() {
    println!(
        "Vdora\n\nUsage:\n  vdora                       Launch the GUI\n  vdora --oneshot             Record until toggled, transcribe, copy\n  vdora voice                 Alias for --oneshot\n\nOptions:\n      --no-notify             Disable desktop notifications\n  -h, --help                  Show this help\n\nOneshot behavior:\n  Press the shortcut once to start recording. Press it again to stop."
    );
}

pub fn run(options: Options) -> Result<()> {
    let paths = Paths::new()?;
    let notifications = !options.no_notify;
    let current_pid = std::process::id();
    handle_existing_run_or_exit(&paths, notifications, current_pid)?;
    let _state_cleanup = StateCleanup {
        paths: &paths,
        current_pid,
    };

    let config = AppConfig::load_or_default();
    ensure_model(&config.model_path)?;

    let recorder = Recorder::new();

    eprintln!("Recording. Run vdora --oneshot again to stop.");
    notify(
        &paths,
        notifications,
        "Recording. Press shortcut again to stop.",
    );

    let session = recorder.start()?;
    write_state(
        &paths,
        &State {
            parent_pid: Some(current_pid),
            recorder_pid: Some(session.recorder_pid()),
            phase: Some("recording".to_string()),
        },
    )?;

    // Model loading is hidden behind recording time instead of delaying the stop action.
    let mut service = match WhisperService::load(config.model_path.clone()) {
        Ok(service) => service,
        Err(err) => {
            let _ = session.stop();
            let _ = cleanup_state(&paths, current_pid);
            notify(&paths, notifications, "Unable to load Whisper model.");
            return Err(err);
        }
    };

    let audio = session.wait()?;

    write_state(
        &paths,
        &State {
            parent_pid: Some(current_pid),
            recorder_pid: None,
            phase: Some("transcribing".to_string()),
        },
    )?;
    eprintln!("Transcribing...");
    notify(&paths, notifications, "Transcribing...");

    let transcript = service.transcribe_file(audio.path(), config.language.as_deref())?;

    if transcript.trim().is_empty() {
        cleanup_state(&paths, current_pid)?;
        notify(&paths, notifications, "No speech detected.");
        return Err(anyhow!("no speech detected"));
    }

    clipboard::set_text(&transcript).context("failed to copy transcript to clipboard")?;

    if config.autopaste {
        if let Err(err) = paste::trigger(config.paste_mode) {
            tracing::warn!("auto-paste failed after clipboard copy: {err}");
            notify(
                &paths,
                notifications,
                "Transcript copied. Auto-paste failed.",
            );
        } else {
            notify(&paths, notifications, "Transcript copied and pasted.");
        }
    } else {
        notify(&paths, notifications, "Transcript copied to clipboard.");
    }

    eprintln!(
        "Transcript copied to clipboard. Manual paste shortcut: {}.",
        config.paste_mode.shortcut_label()
    );
    cleanup_state(&paths, current_pid)?;
    Ok(())
}

impl Paths {
    fn new() -> Result<Self> {
        let runtime_base = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let runtime_dir = runtime_base.join("vdora").join("oneshot");
        Ok(Self {
            state_file: runtime_dir.join("state"),
            lock_file: runtime_dir.join("lock"),
            notification_id_file: runtime_dir.join("notification-id"),
            runtime_dir,
        })
    }
}

fn handle_existing_run_or_exit(paths: &Paths, notifications: bool, current_pid: u32) -> Result<()> {
    fs::create_dir_all(&paths.runtime_dir)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&paths.lock_file)?;
    lock.lock()?;

    let state = read_state(paths)?;
    if let Some(parent_pid) = state.parent_pid {
        if process_exists(parent_pid) {
            if state.phase.as_deref() == Some("recording") {
                if let Some(recorder_pid) = state.recorder_pid {
                    if process_exists(recorder_pid) {
                        let _ = kill(Pid::from_raw(recorder_pid as i32), Signal::SIGINT);
                        eprintln!("Stopped recording. Transcription will start now.");
                        notify(paths, notifications, "Stopped recording. Transcribing...");
                        std::process::exit(0);
                    }
                }
            }

            let phase = state.phase.unwrap_or_else(|| "running".to_string());
            eprintln!("Vdora is already {phase}. Wait for it to finish.");
            notify(
                paths,
                notifications,
                &format!("Already {phase}. Wait for it to finish."),
            );
            std::process::exit(0);
        }
    }

    let _ = fs::remove_file(&paths.state_file);
    write_state(
        paths,
        &State {
            parent_pid: Some(current_pid),
            recorder_pid: None,
            phase: Some("starting".to_string()),
        },
    )?;
    lock.unlock()?;
    Ok(())
}

fn process_exists(pid: u32) -> bool {
    kill(Pid::from_raw(pid as i32), None).is_ok()
}

fn read_state(paths: &Paths) -> Result<State> {
    let mut state = State::default();
    let Ok(mut file) = File::open(&paths.state_file) else {
        return Ok(state);
    };

    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    for line in contents.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key {
            "parent_pid" => state.parent_pid = parse_pid(value),
            "recorder_pid" => state.recorder_pid = parse_pid(value),
            "phase" => state.phase = Some(value.to_string()),
            _ => {}
        }
    }
    Ok(state)
}

fn parse_pid(value: &str) -> Option<u32> {
    match value.parse::<u32>().ok()? {
        0 => None,
        pid => Some(pid),
    }
}

fn write_state(paths: &Paths, state: &State) -> Result<()> {
    fs::create_dir_all(&paths.runtime_dir)?;
    let mut file = tempfile::NamedTempFile::new_in(&paths.runtime_dir)?;
    writeln!(file, "parent_pid={}", state.parent_pid.unwrap_or_default())?;
    writeln!(
        file,
        "recorder_pid={}",
        state.recorder_pid.unwrap_or_default()
    )?;
    writeln!(file, "phase={}", state.phase.as_deref().unwrap_or_default())?;
    file.persist(&paths.state_file).map_err(|err| err.error)?;
    Ok(())
}

fn cleanup_state(paths: &Paths, current_pid: u32) -> Result<()> {
    let state = read_state(paths)?;
    if state.parent_pid == Some(current_pid) {
        let _ = fs::remove_file(&paths.state_file);
    }
    Ok(())
}

// Clear the active-run marker on every exit path, including recording,
// transcription, download, and clipboard failures.
struct StateCleanup<'a> {
    paths: &'a Paths,
    current_pid: u32,
}

impl Drop for StateCleanup<'_> {
    fn drop(&mut self) {
        if let Err(err) = cleanup_state(self.paths, self.current_pid) {
            tracing::warn!("failed to clean up oneshot state: {err:#}");
        }
    }
}

fn notify(paths: &Paths, enabled: bool, message: &str) {
    if !enabled || which::which("notify-send").is_err() {
        return;
    }

    let _ = fs::create_dir_all(&paths.runtime_dir);
    let previous_id = fs::read_to_string(&paths.notification_id_file)
        .ok()
        .map(|raw| raw.trim().to_string())
        .filter(|id| !id.is_empty());

    let mut cmd = Command::new("notify-send");
    cmd.arg("--print-id")
        .arg("--app-name=vdora")
        .arg("--transient")
        .arg("--expire-time=5000");

    if let Some(id) = previous_id {
        cmd.arg(format!("--replace-id={id}"));
    }

    let output = cmd.arg(APP_TITLE).arg(message).output();
    if let Ok(output) = output {
        if output.status.success() {
            if let Ok(id) = String::from_utf8(output.stdout) {
                let id = id.trim();
                if !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()) {
                    let _ = fs::write(&paths.notification_id_file, format!("{id}\n"));
                }
            }
        }
    }
}

fn ensure_model(model_path: &Path) -> Result<()> {
    if model_path.exists() {
        return crate::stt::whisper::validate_model_path(model_path);
    }

    let Some(file_name) = model_path.file_name().and_then(|name| name.to_str()) else {
        return Err(anyhow!("invalid model path: {}", model_path.display()));
    };

    if file_name != DEFAULT_MODEL_FILE {
        return Err(anyhow!(
            "missing model file at {}. Only the default model is auto-downloaded.",
            model_path.display()
        ));
    }

    if let Some(parent) = model_path.parent() {
        fs::create_dir_all(parent)?;
    }

    eprintln!(
        "Downloading default Whisper model to {}",
        model_path.display()
    );
    let response = ureq::get(DEFAULT_MODEL_URL)
        .call()
        .context("failed to download default Whisper model")?;
    let mut reader = response.into_body().into_reader();
    // Unique, same-directory temporary files prevent concurrent downloads from
    // sharing a partial model; failed downloads are removed automatically.
    let parent = model_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut out = tempfile::NamedTempFile::new_in(parent)?;
    let bytes = std::io::copy(&mut reader, &mut out)?;
    if bytes == 0 {
        return Err(anyhow!("downloaded model is empty"));
    }
    out.as_file().sync_all()?;
    out.persist(model_path).map_err(|err| err.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Paths, State, StateCleanup, ensure_model, read_state, write_state};

    fn paths(dir: &std::path::Path) -> Paths {
        Paths {
            runtime_dir: dir.to_path_buf(),
            state_file: dir.join("state"),
            lock_file: dir.join("lock"),
            notification_id_file: dir.join("notification-id"),
        }
    }

    #[test]
    fn file_lock_excludes_other_handles_until_unlocked() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let first = file.reopen().unwrap();
        let second = file.reopen().unwrap();
        first.lock().unwrap();
        assert!(matches!(
            second.try_lock(),
            Err(std::fs::TryLockError::WouldBlock)
        ));
        first.unlock().unwrap();
        second.try_lock().unwrap();
        second.unlock().unwrap();
    }

    #[test]
    fn state_is_cleaned_on_error_return() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(dir.path());
        write_state(
            &paths,
            &State {
                parent_pid: Some(123),
                phase: Some("transcribing".into()),
                ..State::default()
            },
        )
        .unwrap();
        let fail = || -> anyhow::Result<()> {
            let _cleanup = StateCleanup {
                paths: &paths,
                current_pid: 123,
            };
            Err(anyhow::anyhow!("transcription failed"))
        };
        assert!(fail().is_err());
        assert!(!paths.state_file.exists());
    }

    #[test]
    fn cleanup_does_not_remove_another_runs_state() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(dir.path());
        write_state(
            &paths,
            &State {
                parent_pid: Some(456),
                ..State::default()
            },
        )
        .unwrap();
        drop(StateCleanup {
            paths: &paths,
            current_pid: 123,
        });
        assert_eq!(read_state(&paths).unwrap().parent_pid, Some(456));
    }

    #[test]
    fn existing_model_directory_is_rejected_without_download() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            ensure_model(dir.path())
                .unwrap_err()
                .to_string()
                .contains("regular file")
        );
    }
}
