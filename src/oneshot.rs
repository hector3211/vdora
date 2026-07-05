use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use anyhow::{Context, Result, anyhow};
use fs2::FileExt;
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
const DEFAULT_DURATION_SECS: u64 = 30;
const DEFAULT_MODEL_FILE: &str = "ggml-base.en.bin";
const DEFAULT_MODEL_URL: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin";

#[derive(Debug, Clone)]
pub struct Options {
    pub duration: Duration,
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
    let mut duration = DEFAULT_DURATION_SECS;
    let mut no_notify = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--oneshot" | "voice" => oneshot = true,
            "--duration" | "-d" => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow!("{arg} requires a value in seconds"))?;
                duration = value
                    .parse::<u64>()
                    .with_context(|| format!("invalid duration: {value}"))?;
            }
            "--no-notify" => no_notify = true,
            "--help" | "-h" => return Ok(Mode::Help),
            unknown => return Err(anyhow!("unknown argument: {unknown}")),
        }
    }

    if !oneshot {
        return Ok(Mode::Gui);
    }

    if duration == 0 {
        return Err(anyhow!("duration must be greater than zero"));
    }

    Ok(Mode::Run(Options {
        duration: Duration::from_secs(duration),
        no_notify,
    }))
}

pub fn print_help() {
    println!(
        "Vdora\n\nUsage:\n  vdora                       Launch the GUI\n  vdora --oneshot [-d 30]     Record for up to 30s, transcribe, copy\n  vdora voice [-d 30]         Alias for --oneshot\n\nOptions:\n  -d, --duration SECONDS      Maximum recording length, default 30\n      --no-notify             Disable desktop notifications\n  -h, --help                  Show this help\n\nOneshot behavior:\n  Press shortcut once to start recording. Run the same command again to stop early."
    );
}

pub fn run(options: Options) -> Result<()> {
    let paths = Paths::new()?;
    let notifications = !options.no_notify;
    handle_existing_run_or_exit(&paths, notifications)?;

    let config = AppConfig::load_or_default();
    ensure_model(&config.model_path)?;

    let recorder = Recorder::new();
    let current_pid = std::process::id();

    eprintln!(
        "Recording up to {} seconds. Run vdora --oneshot again to stop early.",
        options.duration.as_secs()
    );
    notify(
        &paths,
        notifications,
        &format!(
            "Recording up to {}s. Press shortcut again to stop.",
            options.duration.as_secs()
        ),
    );

    let session = recorder.start_with_max_duration(options.duration)?;
    write_state(
        &paths,
        &State {
            parent_pid: Some(current_pid),
            recorder_pid: Some(session.recorder_pid()),
            phase: Some("recording".to_string()),
        },
    )?;

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

    let service = WhisperService::new(config.model_path.clone(), config.language.clone());
    let transcript = service.transcribe_file(audio.path())?;

    if transcript.trim().is_empty() {
        cleanup_state(&paths, current_pid)?;
        notify(&paths, notifications, "No speech detected.");
        return Err(anyhow!("no speech detected"));
    }

    clipboard::set_text(&transcript).context("failed to copy transcript to clipboard")?;

    if config.autopaste {
        if let Err(err) = paste::trigger_ctrl_v() {
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

    eprintln!("Transcript copied to clipboard. Paste it into opencode with Ctrl+V.");
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

fn handle_existing_run_or_exit(paths: &Paths, notifications: bool) -> Result<()> {
    fs::create_dir_all(&paths.runtime_dir)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&paths.lock_file)?;
    lock.lock_exclusive()?;

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
    let mut file = File::create(&paths.state_file)?;
    writeln!(file, "parent_pid={}", state.parent_pid.unwrap_or_default())?;
    writeln!(
        file,
        "recorder_pid={}",
        state.recorder_pid.unwrap_or_default()
    )?;
    writeln!(file, "phase={}", state.phase.as_deref().unwrap_or_default())?;
    Ok(())
}

fn cleanup_state(paths: &Paths, current_pid: u32) -> Result<()> {
    let state = read_state(paths)?;
    if state.parent_pid == Some(current_pid) {
        let _ = fs::remove_file(&paths.state_file);
    }
    Ok(())
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
        return Ok(());
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
    let tmp_path = model_path.with_extension("bin.tmp");
    let mut out = File::create(&tmp_path)?;
    std::io::copy(&mut reader, &mut out)?;
    fs::rename(&tmp_path, model_path)?;
    Ok(())
}
