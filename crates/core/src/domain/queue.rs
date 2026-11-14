use crate::deps::serde::{Deserialize, Serialize};
use crate::domain::ids::TrackId;
use crate::domain::playback::RepeatMode;

/// 播放队列实体
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Queue {
    pub items: Vec<TrackId>,
    pub cursor: Option<usize>,
    pub version: u64,
    pub repeat_mode: RepeatMode,
    pub shuffle: bool,
}

impl Queue {
    /// 创建空播放队列
    pub fn nw() -> Self {
        Self::default()
    }

    /// 追加曲目到队列尾部
    pub fn psh(&mut self, id: TrackId) {
        self.items.push(id);
        self.version = self.version.wrapping_add(1);
    }

    /// 移除指定索引位置的曲目
    pub fn rmv(&mut self, idx: usize) -> Option<TrackId> {
        if idx >= self.items.len() {
            return None;
        }
        let rmvd = self.items.remove(idx);
        if let Some(c) = self.cursor {
            if idx < c {
                self.cursor = Some(c.saturating_sub(1));
            } else if idx == c {
                if self.items.is_empty() {
                    self.cursor = None;
                } else if c >= self.items.len() {
                    self.cursor = Some(0);
                }
            }
        }
        self.version = self.version.wrapping_add(1);
        Some(rmvd)
    }

    /// 清空播放队列
    pub fn clr(&mut self) {
        self.items.clear();
        self.cursor = None;
        self.version = self.version.wrapping_add(1);
    }

    /// 获取当前播放曲目标识
    pub fn crr(&self) -> Option<&TrackId> {
        self.cursor.and_then(|c| self.items.get(c))
    }

    /// 选中指定索引的曲目
    pub fn stcrr(&mut self, idx: usize) -> bool {
        if idx < self.items.len() {
            self.cursor = Some(idx);
            self.version = self.version.wrapping_add(1);
            true
        } else {
            false
        }
    }

    /// 推进至下一首曲目
    pub fn nxt(&mut self) -> Option<&TrackId> {
        if self.items.is_empty() {
            return None;
        }
        let len = self.items.len();
        if self.shuffle && self.repeat_mode != RepeatMode::One {
            use crate::deps::rand::Rng;
            let index = if len == 1 { 0 } else {
                let choices = if self.cursor.is_some() { len - 1 } else { len };
                let candidate = crate::deps::rand::thread_rng().gen_range(0..choices);
                match self.cursor {
                    Some(current) if candidate >= current => candidate + 1,
                    _ => candidate,
                }
            };
            self.stcrr(index);
            return self.crr();
        }
        match self.repeat_mode {
            RepeatMode::One => self.crr(),
            RepeatMode::All => {
                let nxt_idx = match self.cursor {
                    Some(c) => (c + 1) % len,
                    None => 0,
                };
                self.cursor = Some(nxt_idx);
                self.version = self.version.wrapping_add(1);
                self.crr()
            }
            RepeatMode::Off => {
                let nxt_idx = match self.cursor {
                    Some(c) if c + 1 < len => c + 1,
                    None => 0,
                    _ => return None,
                };
                self.cursor = Some(nxt_idx);
                self.version = self.version.wrapping_add(1);
                self.crr()
            }
        }
    }

    /// 回退至上一首曲目
    pub fn prv(&mut self) -> Option<&TrackId> {
        if self.items.is_empty() {
            return None;
        }
        let len = self.items.len();
        let prv_idx = match self.cursor {
            Some(0) | None => len.saturating_sub(1),
            Some(c) => c - 1,
        };
        self.cursor = Some(prv_idx);
        self.version = self.version.wrapping_add(1);
        self.crr()
    }
}
