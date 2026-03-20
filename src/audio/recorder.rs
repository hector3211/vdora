use std::{
    env, fs,
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, SystemTime},
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

pub struct RecordedAudio {
    _temp_path: TempPath,
    path: PathBuf,
}

impl RecordedAudio {
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl RecordingSession {
    pub fn stop(mut self) -> Result<RecordedAudio> {
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
            let temp_path = self
                .temp_path
                .take()
                .ok_or_else(|| anyhow!("recording temp path unexpectedly missing"))?;

            Ok(RecordedAudio {
                _temp_path: temp_path,
                path: self.output_path,
            })
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

pub fn cleanup_stale_recordings(max_age: Duration) -> usize {
    let now = SystemTime::now();
    let mut removed = 0usize;

    let entries = match fs::read_dir(env::temp_dir()) {
        Ok(entries) => entries,
        Err(err) => {
            tracing::warn!("failed to read temp directory for cleanup: {err}");
            return 0;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !is_vdora_recording_path(&path) {
            continue;
        }

        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(err) => {
                tracing::warn!(
                    "failed to read metadata for potential stale recording {}: {err}",
                    path.display()
                );
                continue;
            }
        };

        let modified = match metadata.modified() {
            Ok(modified) => modified,
            Err(err) => {
                tracing::warn!(
                    "failed to read modified time for potential stale recording {}: {err}",
                    path.display()
                );
                continue;
            }
        };

        let age = match now.duration_since(modified) {
            Ok(age) => age,
            Err(_) => continue,
        };

        if age < max_age {
            continue;
        }

        match fs::remove_file(&path) {
            Ok(()) => removed += 1,
            Err(err) => {
                tracing::warn!("failed to remove stale recording {}: {err}", path.display());
            }
        }
    }

    removed
}

fn is_vdora_recording_path(path: &std::path::Path) -> bool {
    let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
        return false;
    };
    name.starts_with("vdora-") && name.ends_with(".wav")
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

#[cfg(test)]
mod tests {
    use super::is_vdora_recording_path;

    #[test]
    fn matches_vdora_temp_recording_names() {
        assert!(is_vdora_recording_path(std::path::Path::new("/tmp/vdora-abc.wav")));
        assert!(is_vdora_recording_path(std::path::Path::new("vdora-123.wav")));
    }

    #[test]
    fn ignores_non_vdora_temp_recordings() {
        assert!(!is_vdora_recording_path(std::path::Path::new("/tmp/vdora-abc.mp3")));
        assert!(!is_vdora_recording_path(std::path::Path::new("/tmp/other-abc.wav")));
        assert!(!is_vdora_recording_path(std::path::Path::new("/tmp/vdora.wav")));
    }
}
