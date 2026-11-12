use crate::deps::serde::{Deserialize, Serialize};

/// 音乐提供商类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProviderId {
    Kugou,
    Local,
}

/// 统一曲目标识
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TrackId {
    pub provider: ProviderId,
    pub raw_id: String,
}

impl TrackId {
    /// 构造酷狗音源标识
    pub fn kg(id: impl Into<String>) -> Self {
        Self {
            provider: ProviderId::Kugou,
            raw_id: id.into().to_ascii_lowercase(),
        }
    }

    /// 构造本地音源标识
    pub fn lcl(id: impl Into<String>) -> Self {
        Self {
            provider: ProviderId::Local,
            raw_id: id.into(),
        }
    }
}
