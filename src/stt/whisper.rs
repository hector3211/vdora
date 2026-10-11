use std::{fs, path::Path, path::PathBuf, time::Instant};

use anyhow::{Context, Result, anyhow};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

const REQUIRED_SAMPLE_RATE: u32 = 16_000;
const MIN_AUDIO_SAMPLES: usize = 4_000;

pub struct WhisperService {
    state: whisper_rs::WhisperState,
}

impl WhisperService {
    pub fn load(model_path: PathBuf) -> Result<Self> {
        validate_model_path(&model_path)?;
        let started_at = Instant::now();
        let context = build_context(&model_path)?;
        let state = context
            .create_state()
            .context("failed to create whisper state")?;
        tracing::debug!(
            "loaded Whisper model {} in {:.2?}",
            model_path.display(),
            started_at.elapsed()
        );
        Ok(Self { state })
    }

    pub fn transcribe_file(&mut self, wav_path: &Path, language: Option<&str>) -> Result<String> {
        let started_at = Instant::now();
        let audio = load_wav_file(wav_path)?;
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        configure_full_params(&mut params);
        params.set_no_context(true);
        params.set_language(language);

        self.state
            .full(params, &audio)
            .context("whisper failed to transcribe audio")?;

        let transcript = collect_segments(&self.state)?;

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
    let mut speech = String::with_capacity(input.len());
    let mut remaining = input;

    while let Some(start) = remaining.find('[') {
        speech.push_str(&remaining[..start]);
        let Some(relative_end) = remaining[start + 1..].find(']') else {
            speech.push_str(&remaining[start..]);
            remaining = "";
            break;
        };
        let end = start + relative_end + 1;
        let marker = &remaining[start..=end];
        if is_non_speech_marker(marker) {
            speech.push(' ');
        } else {
            speech.push_str(marker);
        }
        remaining = &remaining[end + 1..];
    }
    speech.push_str(remaining);

    speech.split_whitespace().collect::<Vec<_>>().join(" ")
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
    let canonical = token
        .trim()
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .map(str::trim)
        .map(|value| value.split_whitespace().collect::<Vec<_>>().join("_"))
        .map(|value| format!("[{}]", value.to_ascii_uppercase()));

    canonical
        .as_deref()
        .is_some_and(|marker| KNOWN_MARKERS.contains(&marker))
}

#[cfg(test)]
mod tests {
    use super::{WhisperService, load_wav_file, normalize_transcript, transcription_thread_count};
    use hound::{SampleFormat, WavSpec, WavWriter};
    use tempfile::{NamedTempFile, TempPath};

    fn write_fixture(
        spec: WavSpec,
        write_samples: impl FnOnce(&mut WavWriter<std::io::BufWriter<std::fs::File>>),
    ) -> TempPath {
        let path = NamedTempFile::new().unwrap().into_temp_path();
        let mut writer = WavWriter::create(&path, spec).unwrap();
        write_samples(&mut writer);
        writer.finalize().unwrap();
        path
    }

    #[test]
    fn loads_pcm_8_16_24_and_32_bit_with_normalized_endpoints() {
        for bits_per_sample in [8, 16, 24, 32] {
            let spec = WavSpec {
                channels: 1,
                sample_rate: 16_000,
                bits_per_sample,
                sample_format: SampleFormat::Int,
            };
            let path = write_fixture(spec, |writer| {
                match bits_per_sample {
                    8 => {
                        writer.write_sample(i8::MIN).unwrap();
                        writer.write_sample(i8::MAX).unwrap();
                    }
                    16 => {
                        writer.write_sample(i16::MIN).unwrap();
                        writer.write_sample(i16::MAX).unwrap();
                    }
                    24 => {
                        writer.write_sample(-8_388_608_i32).unwrap();
                        writer.write_sample(8_388_607_i32).unwrap();
                    }
                    32 => {
                        writer.write_sample(i32::MIN).unwrap();
                        writer.write_sample(i32::MAX).unwrap();
                    }
                    _ => unreachable!(),
                }
                for _ in 2..4_000 {
                    match bits_per_sample {
                        8 => writer.write_sample(0_i8).unwrap(),
                        16 => writer.write_sample(0_i16).unwrap(),
                        24 | 32 => writer.write_sample(0_i32).unwrap(),
                        _ => unreachable!(),
                    }
                }
            });

            let loaded = load_wav_file(&path).unwrap();
            assert_eq!(loaded.len(), 4_000, "{bits_per_sample}-bit length");
            assert!(
                (loaded[0] + 1.0).abs() < 1e-6,
                "{bits_per_sample}-bit negative endpoint"
            );
            assert!(
                loaded[1] <= 1.0 && loaded[1] > 0.99,
                "{bits_per_sample}-bit positive endpoint"
            );
        }
    }

    #[test]
    fn rejects_non_finite_float_samples() {
        let path = write_fixture(
            WavSpec {
                channels: 1,
                sample_rate: 16_000,
                bits_per_sample: 32,
                sample_format: SampleFormat::Float,
            },
            |writer| {
                writer.write_sample(f32::NAN).unwrap();
                for _ in 1..4_000 {
                    writer.write_sample(0.0_f32).unwrap();
                }
            },
        );
        let result = load_wav_file(&path);
        assert!(result.is_err());
    }

    #[test]
    fn rejects_incomplete_interleaved_frames() {
        let path = write_fixture(
            WavSpec {
                channels: 2,
                sample_rate: 16_000,
                bits_per_sample: 16,
                sample_format: SampleFormat::Int,
            },
            |writer| {
                for _ in 0..4_000 {
                    writer.write_sample(0_i16).unwrap();
                }
            },
        );
        let mut bytes = std::fs::read(&path).unwrap();
        let data_size_offset = bytes
            .windows(4)
            .position(|window| window == b"data")
            .unwrap()
            + 4;
        let data_size = u32::from_le_bytes(
            bytes[data_size_offset..data_size_offset + 4]
                .try_into()
                .unwrap(),
        ) + 2;
        bytes[data_size_offset..data_size_offset + 4].copy_from_slice(&data_size.to_le_bytes());
        let riff_size = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) + 2;
        bytes[4..8].copy_from_slice(&riff_size.to_le_bytes());
        bytes.extend_from_slice(&[0, 0]);
        std::fs::write(&path, bytes).unwrap();
        let result = load_wav_file(&path);
        assert!(result.is_err());
    }

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
    fn strips_spaced_case_insensitive_non_speech_markers() {
        let cleaned = normalize_transcript("[ Silence ] hello [inaudible]");
        assert_eq!(cleaned, "hello");
    }

    #[test]
    fn preserves_bracketed_spoken_content() {
        let cleaned = normalize_transcript("use [draft] for the filename");
        assert_eq!(cleaned, "use [draft] for the filename");
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
        hound::SampleFormat::Float => {
            let samples = reader
                .samples::<f32>()
                .collect::<Result<Vec<_>, _>>()
                .context("failed to read float wav samples")?;
            if samples.iter().any(|sample| !sample.is_finite()) {
                return Err(anyhow!("invalid wav file: non-finite float sample"));
            }
            samples
        }
        hound::SampleFormat::Int => {
            if !matches!(spec.bits_per_sample, 8 | 16 | 24 | 32) {
                return Err(anyhow!(
                    "unsupported integer wav bit depth: {}",
                    spec.bits_per_sample
                ));
            }
            let denominator = (1_u64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|sample| sample.map(|value| value as f32 / denominator))
                .collect::<Result<Vec<_>, _>>()
                .context("failed to read int wav samples")?
        }
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
    if samples.len() % channels != 0 {
        return Err(anyhow!(
            "invalid wav file: incomplete interleaved audio frame"
        ));
    }
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
