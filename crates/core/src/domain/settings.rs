use crate::deps::serde::{Deserialize, Serialize};
use crate::domain::playback::RepeatMode;
use crate::domain::track::AudioQuality;

/// 主题选择
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    #[default]
    System,
    Light,
    Dark,
}

/// 窗口关闭行为
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CloseAction {
    Exit,
    #[default]
    Tray,
}

/// 外观偏好
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppearanceSettings {
    pub theme: ThemeMode,
}

/// 音频设置
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioSettings {
    pub quality: AudioQuality,
    pub volume: f32,
    pub repeat: RepeatMode,
    pub shuffle: bool,
}

impl Default for AudioSettings {
    /// 构造默认音频设置。
    fn default() -> Self {
        Self {
            quality: AudioQuality::Lossless,
            volume: 0.5,
            repeat: RepeatMode::Off,
            shuffle: false,
        }
    }
}

/// 窗口偏好
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowSettings {
    pub close_action: CloseAction,
}

/// 歌词设置
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LyricsSettings {
    pub desktop_enabled: bool,
}

/// 缓存预算设置
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CacheSettings {
    pub directory: std::path::PathBuf,
    pub max_disk_mib: u64,
    pub max_artwork_ram_mib: u64,
}

impl Default for CacheSettings {
    /// 构造默认缓存设置。
    fn default() -> Self {
        Self {
            directory: std::path::PathBuf::new(),
            max_disk_mib: 2048,
            max_artwork_ram_mib: 3,
        }
    }
}

/// 下载设置
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DownloadSettings {
    pub directory: std::path::PathBuf,
}

impl Default for DownloadSettings {
    /// 构造默认下载设置。
    fn default() -> Self {
        Self { directory: std::path::PathBuf::new() }
    }
}

/// 客户端全局设置
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub appearance: AppearanceSettings,
    pub audio: AudioSettings,
    pub window: WindowSettings,
    pub lyrics: LyricsSettings,
    pub cache: CacheSettings,
    pub downloads: DownloadSettings,
}
