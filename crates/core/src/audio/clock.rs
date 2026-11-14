use std::sync::atomic::{AtomicU64, Ordering};

/// 播放时钟
pub struct PlaybackClock {
    base_pos_ms: AtomicU64,
}

impl PlaybackClock {
    /// 构造时钟实例
    pub fn nw() -> Self {
        Self {
            base_pos_ms: AtomicU64::new(0),
        }
    }

    /// 设置新的定位基准（毫秒）
    pub fn stbs(&self, ms: u64) {
        self.base_pos_ms.store(ms, Ordering::Release);
    }

    /// 计算当前真实播放进度（毫秒）
    pub fn pos(&self, played_frames: u64, sample_rate: u32) -> u64 {
        let base = self.base_pos_ms.load(Ordering::Acquire);
        if sample_rate == 0 {
            return base;
        }
        let elapsed_ms = (played_frames as u128 * 1000) / sample_rate as u128;
        base + elapsed_ms as u64
    }
}
