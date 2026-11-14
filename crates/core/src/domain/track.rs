use std::collections::BTreeMap;
use crate::deps::serde::{Deserialize, Serialize};
use crate::domain::ids::TrackId;

/// 音频音质等级
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
pub enum AudioQuality {
    Standard,
    High,
    #[default]
    Lossless,
    HiRes,
}

impl AudioQuality {
    /// 判断是否为无损规格。
    pub fn islss(self) -> bool {
        matches!(self, Self::Lossless | Self::HiRes)
    }

    /// 转换为酷狗接口音质标识。
    pub fn kgstr(self) -> &'static str {
        match self {
            Self::Standard => "128",
            Self::High => "320",
            Self::Lossless => "flac",
            Self::HiRes => "high",
        }
    }

    /// 解析存储或接口中的音质标识。
    pub fn frmkg(raw: &str) -> Option<Self> {
        match raw {
            "128" => Some(Self::Standard),
            "320" => Some(Self::High),
            "flac" => Some(Self::Lossless),
            "high" => Some(Self::HiRes),
            _ => None,
        }
    }

    /// 返回界面展示名称。
    pub fn qltlbl(self) -> &'static str {
        match self {
            Self::Standard => "标准MP3",
            Self::High => "高品MP3",
            Self::Lossless => "无损FLAC",
            Self::HiRes => "Hires无损FLAC",
        }
    }
}

/// 完整曲目实体
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Track {
    pub id: TrackId,
    pub title: String,
    pub artists: Vec<String>,
    pub album: Option<String>,
    #[serde(default)]
    pub album_id: Option<String>,
    #[serde(default)]
    pub album_audio_id: Option<i64>,
    pub duration_ms: Option<u64>,
    pub cover_ref: Option<String>,
    pub qualities: Vec<AudioQuality>,
    #[serde(default)]
    pub extra_hashes: BTreeMap<String, String>,
}

impl Track {
    /// 提取UI精简概要
    pub fn tosmry(&self) -> TrackSummary {
        TrackSummary {
            id: self.id.clone(),
            title: self.title.clone(),
            artist_text: self.artists.join(" / "),
            cover_key: self.cover_ref.clone(),
            duration_ms: self.duration_ms,
            album_id: self.album_id.clone(),
            album_name: self.album.clone(),
            album_audio_id: self.album_audio_id,
            extra_hashes: self.extra_hashes.clone(),
        }
    }
}

/// UI展示与列表精简模型
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackSummary {
    pub id: TrackId,
    pub title: String,
    pub artist_text: String,
    pub cover_key: Option<String>,
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub album_id: Option<String>,
    #[serde(default)]
    pub album_name: Option<String>,
    #[serde(default)]
    pub album_audio_id: Option<i64>,
    #[serde(default)]
    pub extra_hashes: BTreeMap<String, String>,
}

impl TrackSummary {
    /// 返回“歌手 - 标题”形式的名称，缺歌手时仅返回标题。
    pub fn colttl(&self) -> String {
        if self.artist_text.is_empty() { self.title.clone() } else { format!("{} - {}", self.artist_text, self.title) }
    }

    /// 返回下载集合名称，优先使用专辑名。
    pub fn collname(&self) -> String {
        self.album_name.clone().unwrap_or_else(|| self.colttl())
    }

    /// 构造带默认扩展元数据的曲目概要。
    pub fn nw(id: TrackId, title: impl Into<String>, artist_text: impl Into<String>) -> Self {
        Self {
            id,
            title: title.into(),
            artist_text: artist_text.into(),
            cover_key: None,
            duration_ms: None,
            album_id: None,
            album_name: None,
            album_audio_id: None,
            extra_hashes: BTreeMap::new(),
        }
    }
}

/// CoverFlow 卡片类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeckKind {
    Like,
    DailyRecommend,
    Album,
    Single,
    UserPlaylist,
    RecommendedPlaylist,
}

/// CoverFlow 中的单个播放列表卡片。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverDeck {
    pub id: String,
    pub kind: DeckKind,
    pub title: String,
    pub cover_key: Option<String>,
    pub source_id: String,
    pub removable: bool,
    pub tracks_loaded: bool,
    pub tracks: Vec<TrackSummary>,
    pub cursor: Option<usize>,
}

/// 用户歌单与推荐歌单摘要信息。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserPlaylistMeta {
    pub id: String,
    pub list_id: Option<i64>,
    pub global_id: Option<String>,
    pub name: String,
    pub cover_key: Option<String>,
    pub song_count: u32,
    pub is_own: bool,
    pub is_like: bool,
}

/// 去除酷狗歌单条目标题末尾携带的音频文件扩展名。
pub fn clnttl(name: &str) -> String {
    const EXTS: [&str; 6] = ["mp3", "flac", "m4a", "wav", "ogg", "aac"];
    let trimmed = name.trim();
    if let Some((stem, ext)) = trimmed.rsplit_once('.') {
        if EXTS.iter().any(|known| known.eq_ignore_ascii_case(ext.trim())) && !stem.trim().is_empty() {
            return stem.trim_end().to_string();
        }
    }
    trimmed.to_string()
}

/// 本地已完成下载的音频索引记录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadedFile {
    pub track_id: TrackId,
    pub quality: AudioQuality,
    pub is_lossless: bool,
    pub collection_name: String,
    pub file_path: String,
}

/// 解析后的音频源
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedSource {
    pub url_or_local: String,
    pub codec_hint: Option<String>,
}

/// 单行歌词项
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LyricLine {
    pub begin_ms: u64,
    pub end_ms: Option<u64>,
    pub text: String,
}
