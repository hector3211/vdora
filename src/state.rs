#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppState {
    Idle,
    Recording,
    Transcribing,
    Error,
}

impl AppState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::Recording => "Recording...",
            Self::Transcribing => "Transcribing...",
            Self::Error => "Error",
        }
    }
}
