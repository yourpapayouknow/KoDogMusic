use crate::application::commands::AppRoute;
use crate::deps::serde::{Deserialize, Serialize};
use crate::domain::account::{AccountState, CheckInState};
use crate::domain::playback::{PlaybackState, RepeatMode};
use crate::domain::settings::{AppSettings, ThemeMode};
use crate::domain::track::{AudioQuality, DeckKind, TrackSummary, UserPlaylistMeta};

/// CoverFlow 单张封面摘要。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeckSummary {
    pub id: String,
    pub kind: DeckKind,
    pub title: String,
    pub cover_key: Option<String>,
    pub removable: bool,
    pub tracks_loaded: bool,
    pub track_count: usize,
}

/// 收藏页二级歌单详情视图。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenedPlaylist {
    pub meta: UserPlaylistMeta,
    pub tracks: Vec<TrackSummary>,
}

/// 专辑视图页状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlbumView {
    pub title: String,
    pub source_id: String,
    pub tracks: Vec<TrackSummary>,
}

/// 「添加至」二级弹窗状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddToPrompt {
    pub title: String,
    pub cover_key: Option<String>,
    pub source_id: String,
    pub whole_album: bool,
    pub tracks: Vec<TrackSummary>,
    pub created_playlist_id: Option<String>,
}

/// 下载品质二级选择弹窗状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadPrompt {
    pub collection_name: String,
    pub tracks: Vec<TrackSummary>,
    pub available: Vec<AudioQuality>,
}

/// 面向 UI 的全局完整状态快照
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct UiSnapshot {
    pub route: AppRoute,
    pub account: AccountState,
    pub login_qr: Option<crate::domain::account::LoginQr>,
    pub checkin: CheckInState,
    pub playback: PlaybackState,
    pub current: Option<TrackSummary>,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub volume: f32,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    pub search_results: Vec<TrackSummary>,
    pub favorites: Vec<TrackSummary>,
    pub user_playlists: Vec<UserPlaylistMeta>,
    pub opened_playlist: Option<OpenedPlaylist>,
    pub decks: Vec<DeckSummary>,
    pub active_deck: Option<usize>,
    pub deck_flow_seq: u64,
    pub login_flow_seq: u64,
    pub add_to_prompt: Option<AddToPrompt>,
    pub confirm_unlike: Option<String>,
    pub download_prompt: Option<DownloadPrompt>,
    pub queue: Vec<TrackSummary>,
    pub queue_cursor: Option<usize>,
    pub download: Option<crate::services::downloads::DownloadJob>,
    pub theme: ThemeMode,
    pub settings: AppSettings,
    pub is_favorite: bool,
    pub favorites_local: bool,
    pub lyric_text: String,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub album_view: Option<AlbumView>,
}
