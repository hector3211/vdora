use std::{fs, path::Path, path::PathBuf, time::Instant};

use anyhow::{Context, Result, anyhow};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

const REQUIRED_SAMPLE_RATE: u32 = 16_000;
const MIN_AUDIO_SAMPLES: usize = 4_000;

pub struct WhisperService {
    context: WhisperContext,
}

impl WhisperService {
    pub fn load(model_path: PathBuf) -> Result<Self> {
        validate_model_path(&model_path)?;
        let started_at = Instant::now();
        let context = build_context(&model_path)?;
        tracing::debug!(
            "loaded Whisper model {} in {:.2?}",
            model_path.display(),
            started_at.elapsed()
        );
        Ok(Self { context })
    }

    pub fn transcribe_file(&self, wav_path: &Path, language: Option<&str>) -> Result<String> {
        let started_at = Instant::now();
        let audio = load_wav_file(wav_path)?;
        let mut state = self
            .context
            .create_state()
            .context("failed to create whisper state")?;
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        configure_full_params(&mut params);
        if let Some(language) = language {
            params.set_language(Some(language));
        }

        state
            .full(params, &audio)
            .context("whisper failed to transcribe audio")?;

        let transcript = collect_segments(&state)?;

        let cleaned = normalize_transcript(&transcript);
        tracing::debug!(
            "transcribed {} in {:.2?}",
            wav_path.display(),
            started_at.elapsed()
        );
        Ok(cleaned)
    }
}

pub fn validate_model_path(path: &Path) -> Result<()> {
    if !path.exists() {
        return Err(anyhow!("missing model file at {}", path.display()));
    }

    let metadata = fs::metadata(path)
        .with_context(|| format!("failed to read model metadata at {}", path.display()))?;
    if !metadata.is_file() {
        return Err(anyhow!("model path must point to a regular file"));
    }
    Ok(())
}

fn build_context(model_path: &Path) -> Result<WhisperContext> {
    WhisperContext::new_with_params(
        model_path
            .to_str()
            .ok_or_else(|| anyhow!("invalid model path"))?,
        WhisperContextParameters::default(),
    )
    .context("failed to initialize whisper context")
}

fn configure_full_params(params: &mut FullParams<'_, '_>) {
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_translate(false);
    let available_threads = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(4);
    params.set_n_threads(transcription_thread_count(available_threads));
}

fn transcription_thread_count(available_threads: usize) -> i32 {
    available_threads.clamp(1, 8) as i32
}

fn collect_segments(state: &whisper_rs::WhisperState) -> Result<String> {
    let segments = state.full_n_segments();
    let mut transcript = String::new();
    for idx in 0..segments {
        let segment = state
            .get_segment(idx)
            .ok_or_else(|| anyhow!("segment index {idx} out of bounds"))?;
        transcript.push_str(
            segment
                .to_str_lossy()
                .map_err(|err| anyhow!("failed to read segment text: {err}"))?
                .trim(),
        );
        transcript.push(' ');
    }
    Ok(transcript)
}

fn normalize_transcript(input: &str) -> String {
    input
        .split_whitespace()
        .filter(|token| !is_non_speech_marker(token))
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_non_speech_marker(token: &str) -> bool {
    const KNOWN_MARKERS: [&str; 6] = [
        "[BLANK_AUDIO]",
        "[MUSIC]",
        "[NOISE]",
        "[LAUGHTER]",
        "[SILENCE]",
        "[INAUDIBLE]",
    ];
    KNOWN_MARKERS.contains(&token)
}

#[cfg(test)]
mod tests {
    use super::{WhisperService, normalize_transcript, transcription_thread_count};

    #[test]
    fn transcription_threads_scale_without_oversubscribing_large_machines() {
        assert_eq!(transcription_thread_count(1), 1);
        assert_eq!(transcription_thread_count(4), 4);
        assert_eq!(transcription_thread_count(16), 8);
    }

    #[test]
    fn loading_service_rejects_a_missing_model_immediately() {
        let missing = std::env::temp_dir().join("vdora-missing-whisper-model.bin");
        let err = match WhisperService::load(missing) {
            Ok(_) => panic!("loading should fail before transcription starts"),
            Err(err) => err,
        };

        assert!(err.to_string().contains("missing model file"));
    }

    #[test]
    fn normalizes_whitespace() {
        let cleaned = normalize_transcript(" hello\n\tworld   from  vdora ");
        assert_eq!(cleaned, "hello world from vdora");
    }

    #[test]
    fn strips_non_speech_markers() {
        let cleaned = normalize_transcript("[BLANK_AUDIO] hello [MUSIC] world");
        assert_eq!(cleaned, "hello world");
    }

    #[test]
    fn keeps_empty_empty() {
        let cleaned = normalize_transcript("   \n\t  ");
        assert_eq!(cleaned, "");
    }
}

fn load_wav_file(path: &Path) -> Result<Vec<f32>> {
    let mut reader = hound::WavReader::open(path)
        .with_context(|| format!("failed to open wav file at {}", path.display()))?;
    let spec = reader.spec();

    if spec.sample_rate != REQUIRED_SAMPLE_RATE {
        return Err(anyhow!(
            "invalid wav sample rate: expected {}Hz, got {}Hz",
            REQUIRED_SAMPLE_RATE,
            spec.sample_rate
        ));
    }

    let samples = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<Result<Vec<_>, _>>()
            .context("failed to read float wav samples")?,
        hound::SampleFormat::Int => reader
            .samples::<i16>()
            .map(|s| s.map(|v| v as f32 / i16::MAX as f32))
            .collect::<Result<Vec<_>, _>>()
            .context("failed to read int wav samples")?,
    };

    if spec.channels == 0 {
        return Err(anyhow!("invalid wav file: zero channels"));
    }

    if spec.channels == 1 {
        if samples.len() < MIN_AUDIO_SAMPLES {
            return Err(anyhow!(
                "recording too short: need at least {}ms of audio",
                MIN_AUDIO_SAMPLES * 1000 / REQUIRED_SAMPLE_RATE as usize
            ));
        }
        return Ok(samples);
    }

    let channels = spec.channels as usize;
    let mono: Vec<f32> = samples
        .chunks(channels)
        .map(|frame| frame.iter().copied().sum::<f32>() / channels as f32)
        .collect();

    if mono.len() < MIN_AUDIO_SAMPLES {
        return Err(anyhow!(
            "recording too short: need at least {}ms of audio",
            MIN_AUDIO_SAMPLES * 1000 / REQUIRED_SAMPLE_RATE as usize
        ));
    }

    Ok(mono)
}
