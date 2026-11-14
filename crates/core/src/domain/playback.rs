use crate::deps::serde::{Deserialize, Serialize};

/// 播放引擎状态机
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PlaybackState {
    #[default]
    Idle,
    Resolving,
    Buffering,
    Playing,
    Paused,
    Seeking,
    Recovering,
    Ended,
    Failed,
}

/// 播放循环模式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum RepeatMode {
    #[default]
    Off,
    All,
    One,
}

/// 播放快照信息
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaybackProgress {
    pub position_ms: u64,
    pub duration_ms: u64,
    pub volume: f32,
    pub state: PlaybackState,
}
