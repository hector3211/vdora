use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, bail};
use cpal::{
    FromSample, SampleFormat, SizedSample, Stream, StreamConfig,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};

use super::{SAMPLE_RATE, resample::Resampler};

pub const MAX_SECONDS: usize = 120;
const MAX_SAMPLES: usize = MAX_SECONDS * SAMPLE_RATE as usize;

pub struct Capture {
    stream: Stream,
    data: Arc<Mutex<CaptureData>>,
}

struct CaptureData {
    samples: Vec<f32>,
    error: Option<String>,
}

impl Capture {
    pub fn start() -> Result<Self> {
        let device = cpal::default_host()
            .default_input_device()
            .context("no default microphone; check Windows Sound settings")?;
        let supported = device
            .default_input_config()
            .context("cannot read the default microphone format")?;
        let config = supported.config();
        if config.channels == 0 || config.sample_rate == 0 {
            bail!("invalid microphone channel count or sample rate");
        }
        let data = Arc::new(Mutex::new(CaptureData {
            samples: Vec::with_capacity(MAX_SAMPLES),
            error: None,
        }));
        let stream = match supported.sample_format() {
            SampleFormat::U8 => open_stream::<u8>(&device, config, &data),
            SampleFormat::I16 => open_stream::<i16>(&device, config, &data),
            SampleFormat::I24 => open_stream::<cpal::I24>(&device, config, &data),
            SampleFormat::I32 => open_stream::<i32>(&device, config, &data),
            SampleFormat::I64 => open_stream::<i64>(&device, config, &data),
            SampleFormat::F32 => open_stream::<f32>(&device, config, &data),
            SampleFormat::F64 => open_stream::<f64>(&device, config, &data),
            format => bail!("unsupported microphone format: {format}"),
        }
        .context("cannot open the microphone; check Windows microphone privacy settings")?;
        stream.play().context("cannot start microphone capture")?;
        Ok(Self { stream, data })
    }

    pub fn finish(self) -> Result<Vec<f32>> {
        let Self { stream, data } = self;
        // Dropping the stream stops capture before we take the samples.
        drop(stream);
        let mut data = data
            .lock()
            .map_err(|_| anyhow::anyhow!("microphone capture failed"))?;
        if let Some(err) = data.error.take() {
            bail!("microphone capture failed: {err}");
        }
        Ok(std::mem::take(&mut data.samples))
    }
}

fn open_stream<T>(
    device: &cpal::Device,
    config: StreamConfig,
    data: &Arc<Mutex<CaptureData>>,
) -> Result<Stream, cpal::Error>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = usize::from(config.channels);
    let mut resampler = Resampler::new(config.sample_rate);
    let data_for_callback = Arc::clone(data);
    let data_for_error = Arc::clone(data);
    device.build_input_stream::<T, _, _>(
        config,
        move |input, _| {
            let Ok(mut data) = data_for_callback.lock() else {
                return;
            };
            if data.error.is_some() {
                return;
            }
            for frame in input.chunks_exact(channels) {
                if data.samples.len() >= MAX_SAMPLES {
                    break;
                }
                let mut sum = 0.0;
                for &sample in frame {
                    let sample = sample.to_sample::<f32>();
                    if !sample.is_finite() {
                        data.error = Some("non-finite microphone sample".to_string());
                        return;
                    }
                    sum += sample.clamp(-1.0, 1.0);
                }
                resampler.push(sum / channels as f32, &mut data.samples);
            }
            data.samples.truncate(MAX_SAMPLES);
        },
        move |err| {
            if let Ok(mut data) = data_for_error.lock() {
                data.error.get_or_insert_with(|| err.to_string());
            }
        },
        None,
    )
}
