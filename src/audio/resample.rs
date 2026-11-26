use super::SAMPLE_RATE;

/// Streaming box resampler. Integer phase keeps fractional-rate devices (e.g.
/// 44.1 kHz) sample-accurate even when CPAL delivers split buffers.
pub(super) struct Resampler {
    input_rate: u32,
    remaining: u32,
    weighted_sum: f32,
}

impl Resampler {
    pub(super) fn new(input_rate: u32) -> Self {
        assert!(input_rate > 0);
        Self {
            input_rate,
            remaining: input_rate,
            weighted_sum: 0.0,
        }
    }

    pub(super) fn push(&mut self, sample: f32, output: &mut Vec<f32>) {
        let mut duration = SAMPLE_RATE;
        while duration > 0 {
            let overlap = duration.min(self.remaining);
            self.weighted_sum += sample * overlap as f32;
            duration -= overlap;
            self.remaining -= overlap;
            if self.remaining == 0 {
                output.push(self.weighted_sum / self.input_rate as f32);
                self.weighted_sum = 0.0;
                self.remaining = self.input_rate;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reduces_48khz_to_16khz_by_averaging_each_group() {
        let mut resampler = Resampler::new(48_000);
        let mut output = Vec::new();
        for sample in [1.0, 0.0, -1.0, 0.5, 1.0, 0.0] {
            resampler.push(sample, &mut output);
        }
        assert_eq!(output, [0.0, 0.5]);
    }

    #[test]
    fn fractional_rate_preserves_duration_and_constant_amplitude() {
        let mut resampler = Resampler::new(44_100);
        let mut output = Vec::new();
        for _ in 0..44_100 {
            resampler.push(0.25, &mut output);
        }
        assert_eq!(output.len(), SAMPLE_RATE as usize);
        assert!(output.iter().all(|&sample| sample == 0.25));
    }

    #[test]
    fn handles_a_microphone_rate_below_whispers_input_rate() {
        let mut resampler = Resampler::new(8_000);
        let mut output = Vec::new();
        for sample in [0.25, -0.5] {
            resampler.push(sample, &mut output);
        }
        assert_eq!(output, [0.25, 0.25, -0.5, -0.5]);
    }
}
