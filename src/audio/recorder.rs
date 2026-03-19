use std::{
    path::PathBuf,
    process::{Child, Command, Stdio},
};

use anyhow::{Context, Result, anyhow};
use nix::{
    sys::signal::{Signal, kill},
    unistd::Pid,
};
use tempfile::{Builder, TempPath};

const REQUIRED_SAMPLE_RATE: u32 = 16_000;
const MIN_AUDIO_SAMPLES: u32 = 4_000;

pub struct Recorder;

impl Recorder {
    pub fn new() -> Self {
        Self
    }

    pub fn start(&self) -> Result<RecordingSession> {
        let recorder_binary = locate_recorder_binary()?;
        let temp_file = Builder::new()
            .prefix("vdora-")
            .suffix(".wav")
            .tempfile()
            .context("failed to allocate temporary recording file")?;
        let output_path = temp_file.path().to_path_buf();
        let temp_path = temp_file.into_temp_path();

        let mut cmd = Command::new(recorder_binary);
        cmd.arg("--rate")
            .arg(REQUIRED_SAMPLE_RATE.to_string())
            .arg("--channels")
            .arg("1")
            .arg("--format")
            .arg("s16")
            .arg(&output_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        let child = cmd.spawn().context("failed to start pw-record")?;

        Ok(RecordingSession {
            child,
            output_path,
            temp_path: Some(temp_path),
        })
    }
}

pub struct RecordingSession {
    child: Child,
    output_path: PathBuf,
    temp_path: Option<TempPath>,
}

impl RecordingSession {
    pub fn stop(mut self) -> Result<PathBuf> {
        let pid = Pid::from_raw(self.child.id() as i32);
        if let Err(err) = kill(pid, Signal::SIGINT) {
            tracing::warn!("failed to signal recorder process, continuing wait: {err}");
        }

        let output = self
            .child
            .wait_with_output()
            .context("failed to wait for recorder process")?;

        if recording_file_ready(&self.output_path)? {
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                tracing::warn!(
                    "pw-record exited with status {} but audio file is valid: {}",
                    output.status,
                    stderr.trim()
                );
            }
            if let Some(temp_path) = self.temp_path.take() {
                temp_path
                    .keep()
                    .context("failed to preserve recording for transcription")?;
            }
            Ok(self.output_path)
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(anyhow!(
                "recording failed (status: {}): {}",
                output.status,
                stderr.trim()
            ))
        }
    }
}

fn recording_file_ready(path: &PathBuf) -> Result<bool> {
    let reader = hound::WavReader::open(path)
        .with_context(|| format!("failed to open recording at {}", path.display()))?;
    let spec = reader.spec();

    Ok(spec.channels == 1
        && spec.sample_rate == REQUIRED_SAMPLE_RATE
        && reader.duration() >= MIN_AUDIO_SAMPLES)
}

fn locate_recorder_binary() -> Result<PathBuf> {
    which::which("pw-record")
        .context("pw-record not found. Install PipeWire tools (pipewire-audio-client-libraries).")
}
