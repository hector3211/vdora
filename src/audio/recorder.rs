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

use super::{MIN_SAMPLES, SAMPLE_RATE};

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
            .arg(SAMPLE_RATE.to_string())
            .arg("--channels")
            .arg("1")
            .arg("--format")
            .arg("s16");

        cmd.arg(&output_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        let child = cmd.spawn().context("failed to start pw-record")?;

        Ok(RecordingSession(Some(ActiveRecording { child, temp_path })))
    }
}

pub struct RecordingSession(Option<ActiveRecording>);

struct ActiveRecording {
    child: Child,
    temp_path: TempPath,
}

pub struct RecordedAudio(TempPath);

impl RecordedAudio {
    pub fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl RecordingSession {
    pub fn recorder_pid(&self) -> u32 {
        self.0.as_ref().expect("active recording").child.id()
    }

    pub fn stop(self) -> Result<RecordedAudio> {
        self.request_stop();
        self.finish()
    }

    pub fn request_stop(&self) {
        signal_stop(&self.0.as_ref().expect("active recording").child);
    }

    pub fn finish(mut self) -> Result<RecordedAudio> {
        let ActiveRecording { child, temp_path } = self.0.take().expect("active recording");
        finish_recording(child, temp_path)
    }
}

impl Drop for RecordingSession {
    fn drop(&mut self) {
        if let Some(mut recording) = self.0.take() {
            // An abandoned recording is discarded, so no WAV finalization is needed.
            let _ = recording.child.kill();
            if let Err(err) = recording.child.wait() {
                tracing::warn!("failed to reap abandoned recorder: {err}");
            }
        }
    }
}

fn finish_recording(child: Child, temp_path: TempPath) -> Result<RecordedAudio> {
    let output = wait_for_exit(child)?;
    if recording_file_ready(&temp_path)? {
        log_non_success_if_valid(&output);
        Ok(RecordedAudio(temp_path))
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(anyhow!(
            "recording failed (status: {}): {}",
            output.status,
            stderr.trim()
        ))
    }
}

fn signal_stop(child: &Child) {
    let pid = Pid::from_raw(child.id() as i32);
    if let Err(err) = kill(pid, Signal::SIGINT) {
        tracing::warn!("failed to signal recorder process, continuing wait: {err}");
    }
}

fn wait_for_exit(child: Child) -> Result<std::process::Output> {
    child
        .wait_with_output()
        .context("failed to wait for recorder process")
}

fn log_non_success_if_valid(output: &std::process::Output) {
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::warn!(
            "pw-record exited with status {} but audio file is valid: {}",
            output.status,
            stderr.trim()
        );
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

fn recording_file_ready(path: &std::path::Path) -> Result<bool> {
    let reader = hound::WavReader::open(path)
        .with_context(|| format!("failed to open recording at {}", path.display()))?;
    let spec = reader.spec();

    Ok(spec.channels == 1
        && spec.sample_rate == SAMPLE_RATE
        && reader.duration() as usize >= MIN_SAMPLES)
}

fn locate_recorder_binary() -> Result<PathBuf> {
    which::which("pw-record")
        .context("pw-record not found. Install PipeWire tools (pipewire-audio-client-libraries).")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abandoning_recording_reaps_recorder_and_removes_temporary_file() {
        let temp_path = tempfile::NamedTempFile::new().unwrap().into_temp_path();
        let output_path = temp_path.to_path_buf();
        let child = Command::new("sleep").arg("30").spawn().unwrap();
        let pid = Pid::from_raw(child.id() as i32);
        let session = RecordingSession(Some(ActiveRecording { child, temp_path }));

        drop(session);

        // WNOHANG avoids hanging on a regression, and we still clean up the child.
        use nix::sys::wait::{WaitPidFlag, waitpid};
        let status = waitpid(pid, Some(WaitPidFlag::WNOHANG));
        if matches!(status, Ok(nix::sys::wait::WaitStatus::StillAlive)) {
            kill(pid, Signal::SIGKILL).unwrap();
            waitpid(pid, None).unwrap();
        }
        assert_eq!(status, Err(nix::errno::Errno::ECHILD));
        assert!(!output_path.exists());
    }

    #[test]
    fn matches_vdora_temp_recording_names() {
        assert!(is_vdora_recording_path(std::path::Path::new(
            "/tmp/vdora-abc.wav"
        )));
        assert!(is_vdora_recording_path(std::path::Path::new(
            "vdora-123.wav"
        )));
    }

    #[test]
    fn ignores_non_vdora_temp_recordings() {
        assert!(!is_vdora_recording_path(std::path::Path::new(
            "/tmp/vdora-abc.mp3"
        )));
        assert!(!is_vdora_recording_path(std::path::Path::new(
            "/tmp/other-abc.wav"
        )));
        assert!(!is_vdora_recording_path(std::path::Path::new(
            "/tmp/vdora.wav"
        )));
    }
}
