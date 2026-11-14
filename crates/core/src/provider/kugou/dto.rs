use crate::deps::serde::{Deserialize, Serialize};

/// 搜索响应外层包装
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchSongResp {
    pub status: i32,
    #[serde(default, alias = "errcode", alias = "error_code")]
    pub err_code: i32,
    pub data: Option<SearchSongData>,
}

/// 搜索单曲列表数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchSongData {
    pub total: u64,
    #[serde(alias = "info")]
    pub lists: Vec<SearchSongItem>,
}

/// 搜索单曲条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchSongItem {
    #[serde(rename = "FileHash", alias = "hash")]
    pub file_hash: String,
    #[serde(rename = "SongName", alias = "songname", alias = "OriSongName")]
    pub song_name: String,
    #[serde(rename = "SingerName", alias = "singername")]
    pub singer_name: String,
    #[serde(rename = "AlbumID", alias = "album_id")]
    pub album_id: Option<crate::deps::serde_json::Value>,
    #[serde(rename = "AlbumName", alias = "album_name")]
    pub album_name: Option<String>,
    #[serde(rename = "MixSongID", alias = "AlbumAudioID", alias = "album_audio_id")]
    pub album_audio_id: Option<crate::deps::serde_json::Value>,
    #[serde(rename = "Duration", alias = "duration")]
    pub duration: u64,
    #[serde(rename = "HQFileHash", alias = "320hash")]
    pub hq_file_hash: Option<String>,
    #[serde(rename = "SQFileHash", alias = "sqhash")]
    pub sq_file_hash: Option<String>,
    #[serde(rename = "ResFileHash", alias = "res_hash")]
    pub res_file_hash: Option<String>,
    #[serde(rename = "Image", alias = "img")]
    pub image: Option<String>,
}

