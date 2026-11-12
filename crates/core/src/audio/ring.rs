use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

/// 实时音频环形缓冲
pub struct RealtimeRing {
    samples: Vec<AtomicU32>,
    frames: Vec<AtomicU32>,
    capacity: usize,
    mask: usize,
    read: AtomicUsize,
    write: AtomicUsize,
    clear_before: AtomicUsize,
}

impl RealtimeRing {
    /// 创建环形缓冲
    pub fn nw(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        let physical = capacity.next_power_of_two();
        Self {
            samples: (0..physical).map(|_| AtomicU32::new(0)).collect(),
            frames: (0..physical).map(|_| AtomicU32::new(0)).collect(),
            capacity,
            mask: physical - 1,
            read: AtomicUsize::new(0),
            write: AtomicUsize::new(0),
            clear_before: AtomicUsize::new(0),
        }
    }

    /// 清空缓冲
    pub fn clr(&self) {
        let w = self.write.load(Ordering::Acquire);
        self.clear_before.fetch_max(w, Ordering::Release);
    }

    /// 取缓冲容量（样本数）
    pub fn cap(&self) -> usize {
        self.capacity
    }

    /// 取已缓冲的样本数
    pub fn buflen(&self) -> usize {
        self.write
            .load(Ordering::Acquire)
            .saturating_sub(self.effread(Ordering::Acquire))
            .min(self.capacity)
    }

    /// 判断缓冲是否为空
    pub fn isempty(&self) -> bool {
        self.buflen() == 0
    }

    /// 写入样本，返回实际写入数与写入的源帧数
    pub fn psh(&self, samples: &[f32], src_frames: u64, limit: usize) -> (usize, u64) {
        if samples.is_empty() {
            return (0, 0);
        }
        let read = self.effread(Ordering::Acquire);
        let write = self.write.load(Ordering::Relaxed);
        let avail = limit
            .min(self.capacity)
            .saturating_sub(write.saturating_sub(read));
        let take = avail.min(samples.len());
        if take == 0 {
            return (0, 0);
        }

        let credited = frmspan(src_frames, samples.len(), take);
        let mut stored = 0u64;
        for (i, s) in samples[..take].iter().enumerate() {
            let pos = (write + i) & self.mask;
            self.samples[pos].store(s.to_bits(), Ordering::Relaxed);
            let c = frmcredit(credited, take, i);
            self.frames[pos].store(c, Ordering::Relaxed);
            stored = stored.saturating_add(u64::from(c));
        }
        self.write.store(write + take, Ordering::Release);
        (take, stored)
    }

    /// 读出样本到指定缓冲，返回实际读出数与读出的源帧数
    pub fn pop(&self, out: &mut [f32]) -> (usize, u64) {
        let observed = self.read.load(Ordering::Relaxed);
        let read = observed.max(self.clear_before.load(Ordering::Acquire));
        if read != observed {
            self.read.store(read, Ordering::Release);
        }
        let write = self.write.load(Ordering::Acquire);
        let take = write.saturating_sub(read).min(self.capacity).min(out.len());

        let mut consumed = 0u64;
        for (i, s) in out[..take].iter_mut().enumerate() {
            let pos = (read + i) & self.mask;
            *s = f32::from_bits(self.samples[pos].load(Ordering::Relaxed));
            consumed = consumed.saturating_add(u64::from(self.frames[pos].load(Ordering::Relaxed)));
        }
        self.read.store(read + take, Ordering::Release);
        (take, consumed)
    }

    /// 取生效的读索引
    fn effread(&self, ord: Ordering) -> usize {
        self.read.load(ord).max(self.clear_before.load(ord))
    }
}

/// 按比例计算跨度内的源帧数
fn frmspan(src_frames: u64, total: usize, take: usize) -> u64 {
    if take >= total {
        return src_frames;
    }
    ((src_frames as u128 * take as u128) / total.max(1) as u128) as u64
}

/// 计算单个样本的源帧数信用
fn frmcredit(src_frames: u64, total: usize, index: usize) -> u32 {
    let total = total.max(1) as u128;
    let start = index as u128;
    let end = start + 1;
    let src = src_frames as u128;
    let credit = ((src * end) / total).saturating_sub((src * start) / total);
    u32::try_from(credit).unwrap_or(u32::MAX)
}
