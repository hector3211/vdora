#[cfg(target_os = "linux")]
pub mod recorder;
#[cfg(any(windows, test))]
mod resample;
#[cfg(windows)]
pub mod windows;

pub const SAMPLE_RATE: u32 = 16_000;
pub const MIN_SAMPLES: usize = SAMPLE_RATE as usize / 4;
