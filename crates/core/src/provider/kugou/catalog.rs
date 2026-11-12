use std::collections::{BTreeMap, BTreeSet};
use crate::application::ports::CatalogPort;
use crate::deps::serde_json::Value;
use crate::domain::error::{AppError, NetworkError};
use crate::domain::ids::{ProviderId, TrackId};
use crate::domain::track::{clnttl, AudioQuality, LyricLine, Track, TrackSummary, UserPlaylistMeta};
use crate::provider::kugou::client::KugouClient;
use crate::provider::kugou::library::{fldnum, fldstr, rowtrk};

/// 酷狗媒体目录适配器
pub struct KugouCatalog {
    client: KugouClient,
}

impl KugouCatalog {
    /// 构造适配器
    pub fn nw(client: KugouClient) -> Self {
        Self { client }
    }
}

impl CatalogPort for KugouCatalog {
    /// 获取真实搜索结果并映射为曲目摘要。
    fn srchtrks(&self, qry: &str, pg: u32, lmt: u32) -> Result<Vec<TrackSummary>, AppError> {
        let resp = self.client.srch(qry, pg, lmt)?;

        let mut list = Vec::new();
        if let Some(data) = resp.data {
            for itm in data.lists {
                let album_id = itm.album_id.as_ref().and_then(|v| {
                    v.as_str().map(str::trim).filter(|s| !s.is_empty() && *s != "0").map(ToString::to_string)
                        .or_else(|| v.as_u64().filter(|&n| n > 0).map(|n| n.to_string()))
                });
                let album_audio_id = itm.album_audio_id.as_ref().and_then(|v| {
                    v.as_i64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
                });
                let mut extra_hashes = BTreeMap::new();
                if let Some(h) = itm.hq_file_hash.filter(|s| !s.is_empty()) {
                    extra_hashes.insert("320".into(), h.to_ascii_lowercase());
                }
                if let Some(h) = itm.sq_file_hash.filter(|s| !s.is_empty()) {
                    extra_hashes.insert("flac".into(), h.to_ascii_lowercase());
                }
                if let Some(h) = itm.res_file_hash.filter(|s| !s.is_empty()) {
                    extra_hashes.insert("high".into(), h.to_ascii_lowercase());
                }
                list.push(TrackSummary {
                    id: TrackId::kg(itm.file_hash),
                    title: clnttl(&itm.song_name),
                    artist_text: itm.singer_name,
                    cover_key: itm.image.filter(|image| !image.is_empty()).map(|image| image.replace("{size}", "480")),
                    duration_ms: Some(itm.duration.saturating_mul(1000)),
                    album_id,
                    album_name: itm.album_name.filter(|s| !s.trim().is_empty()),
                    album_audio_id,
                    extra_hashes,
                });
            }
        }
        Ok(list)
    }

    /// 获取曲目详情，本地曲目以文件名作为标题。
    fn trkdtl(&self, id: &TrackId) -> Result<Track, AppError> {
        if id.provider == ProviderId::Local {
            return Ok(Track {
                id: id.clone(),
                title: std::path::Path::new(&id.raw_id).file_stem().map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| id.raw_id.clone()),
                artists: Vec::new(),
                album: None,
                album_id: None,
                album_audio_id: None,
                duration_ms: None,
                cover_ref: None,
                qualities: Vec::new(),
                extra_hashes: BTreeMap::new(),
            });
        }
        Ok(match self.client.trkinf(&id.raw_id) {
            Ok(root) if root.get("status").and_then(Value::as_i64) == Some(1) => {
                let data = root.get("data").unwrap_or(&root);
                let title = fldstr(data, &["songName", "songname"]).map(|name| clnttl(&name)).filter(|name| !name.is_empty());
                Track {
                    id: id.clone(),
                    title: title.unwrap_or_else(|| "未知曲目".into()),
                    artists: fldstr(data, &["singername", "author_name"]).map(|names| names.split("、").map(ToString::to_string).collect()).unwrap_or_default(),
                    album: fldstr(data, &["albumName", "albumname"]).filter(|name| !name.is_empty()),
                    album_id: fldstr(data, &["album_id"]).filter(|name| !name.is_empty() && name != "0"),
                    album_audio_id: fldnum(data, &["album_audio_id"]).and_then(|v| i64::try_from(v).ok()),
                    duration_ms: fldnum(data, &["timeLength"]).map(|secs| secs.saturating_mul(1000)),
                    cover_ref: fldstr(data, &["img", "cover"]).map(|url| url.replace("{size}", "480")).filter(|url| !url.is_empty()),
                    qualities: Vec::new(),
                    extra_hashes: BTreeMap::new(),
                }
            }
            _ => unknown_track(id),
        })
    }

    /// 获取并解析真实歌曲歌词。
    fn lyrc(&self, id: &TrackId) -> Result<Vec<LyricLine>, AppError> {
        if id.provider == ProviderId::Local { return Ok(Vec::new()); }
        Ok(crate::services::lyrics::prslyrc(&self.client.lyrtxt(&id.raw_id)?))
    }

    /// 获取原始LRC歌词文本。
    fn lyrtxt(&self, id: &TrackId) -> Result<String, AppError> {
        if id.provider == ProviderId::Local { return Ok(String::new()); }
        self.client.lyrtxt(&id.raw_id)
    }

    /// 获取每日推荐歌单曲目列表。
    fn dailyrec(&self) -> Result<Vec<TrackSummary>, AppError> {
        let root = self.client.dailyrec()?;
        let data = root.get("data").unwrap_or(&root);
        let rows = data.get("song_list").or_else(|| data.get("songs")).or_else(|| data.get("info"))
            .and_then(Value::as_array)
            .ok_or_else(|| NetworkError::Server("每日推荐响应缺少曲目列表".into()))?;
        Ok(rows.iter().filter_map(|row| rowtrk(row).ok().map(|(t, _)| t)).collect())
    }

    /// 获取上游推荐歌单列表。
    fn recpl(&self, lmt: u32) -> Result<Vec<UserPlaylistMeta>, AppError> {
        let root = self.client.plzpl(1, lmt.max(1))?;
        let rows = root.get("data").and_then(|d| d.get("special_list")).and_then(Value::as_array)
            .ok_or_else(|| NetworkError::Server("推荐歌单响应格式异常".into()))?;
        let mut out = Vec::new();
        for item in rows {
            let Some(gid) = fldstr(item, &["global_collection_id", "global_id"]) else { continue; };
            let Some(name) = fldstr(item, &["specialname", "name", "title"]) else { continue; };
            let cover = fldstr(item, &["flexible_cover", "imgurl", "img", "pic", "cover"])
                .map(|u| u.replace("{size}", "480"));
            let song_count = fldnum(item, &["songcount", "song_count", "count"]).unwrap_or(0) as u32;
            out.push(UserPlaylistMeta {
                id: gid.clone(),
                list_id: fldnum(item, &["specialid"]).and_then(|v| i64::try_from(v).ok()),
                global_id: Some(gid),
                name,
                cover_key: cover,
                song_count,
                is_own: false,
                is_like: false,
            });
        }
        Ok(out)
    }

    /// 获取公开歌单全部歌曲。
    fn pubpltrks(&self, gid: &str) -> Result<Vec<TrackSummary>, AppError> {
        let mut tracks = Vec::new();
        let mut begin = 0usize;
        loop {
            let root = self.client.pubpltrks(gid, begin, 100)?;
            let data = root.get("data").ok_or_else(|| NetworkError::Server("公开歌单缺少数据".into()))?;
            let total = fldnum(data, &["count"]).unwrap_or(0) as usize;
            let rows = data.get("songs").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]);
            if rows.is_empty() { break; }
            begin = begin.saturating_add(rows.len());
            let before = tracks.len();
            for row in rows {
                if let Ok((t, _)) = rowtrk(row) {
                    tracks.push(t);
                }
            }
            if tracks.len() >= total || tracks.len() >= 1800 || tracks.len() == before { break; }
        }
        Ok(tracks)
    }

    /// 获取专辑详情与全部曲目。
    fn albdtl(&self, aid: &str) -> Result<(String, Option<String>, Vec<TrackSummary>), AppError> {
        let detail = self.client.albdtl(aid)?;
        let album = detail.get("data").and_then(Value::as_array).and_then(|a| a.first()).unwrap_or(&Value::Null);
        let album_name = fldstr(album, &["album_name", "name"]).unwrap_or_else(|| format!("专辑 {aid}"));
        let album_cover = fldstr(album, &["sizable_cover", "cover"]).map(|u| u.replace("{size}", "480"));
        let mut tracks = Vec::new();
        let mut page = 1u32;
        loop {
            let root = self.client.albsngs(aid, page, 30)?;
            let data = root.get("data").ok_or_else(|| NetworkError::Server("专辑曲目缺少数据".into()))?;
            let total = fldnum(data, &["total"]).unwrap_or(0) as usize;
            let rows = data.get("songs").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]);
            if rows.is_empty() { break; }
            for row in rows {
                if let Ok((mut t, _)) = rowtrk(row) {
                    t.album_id.get_or_insert_with(|| aid.to_string());
                    t.album_name.get_or_insert_with(|| album_name.clone());
                    if t.cover_key.is_none() { t.cover_key = album_cover.clone(); }
                    tracks.push(t);
                }
            }
            if tracks.len() >= total || rows.len() < 30 || page >= 50 { break; }
            page += 1;
        }
        Ok((album_name, album_cover, tracks))
    }

    /// 探测曲目集合在上游实际提供的下载品质。
    fn prvqlt(&self, trks: &[TrackSummary]) -> Result<Vec<AudioQuality>, AppError> {
        let mut hashes = Vec::new();
        for trk in trks.iter().take(10) {
            if trk.id.provider != ProviderId::Kugou { continue; }
            hashes.push(trk.id.raw_id.clone());
            if trks.len() == 1 {
                for h in trk.extra_hashes.values() {
                    hashes.push(h.clone());
                }
            }
        }
        if hashes.is_empty() {
            return Ok(vec![AudioQuality::High]);
        }
        let root = self.client.prvlite(&hashes)?;
        let avail = prsawqlts(&root);
        let mut out = Vec::new();
        for q in [AudioQuality::High, AudioQuality::Lossless, AudioQuality::HiRes] {
            if avail.contains(&q) { out.push(q); }
        }
        if out.is_empty() && avail.contains(&AudioQuality::Standard) {
            out.push(AudioQuality::Standard);
        }
        if out.is_empty() {
            out.push(AudioQuality::High);
        }
        Ok(out)
    }
}

/// 从 /privilege/lite 响应解析有权限的音质集合。
pub(super) fn prsawqlts(root: &Value) -> BTreeSet<AudioQuality> {
    let mut set = BTreeSet::new();
    let Some(items) = root.get("data").and_then(Value::as_array) else { return set; };
    for item in items {
        let mut check = |entry: &Value| {
            if entry.get("level").and_then(Value::as_i64) == Some(0) { return; }
            if let Some(q) = entry.get("quality").and_then(Value::as_str) {
                match q {
                    "128" => { set.insert(AudioQuality::Standard); }
                    "320" => { set.insert(AudioQuality::High); }
                    "flac" => { set.insert(AudioQuality::Lossless); }
                    "high" => { set.insert(AudioQuality::HiRes); }
                    _ => {}
                }
            }
        };
        check(item);
        if let Some(rel) = item.get("relate_goods").and_then(Value::as_array) {
            for entry in rel { check(entry); }
        }
    }
    set
}

/// 构造缺少元数据的曲目。
fn unknown_track(id: &TrackId) -> Track {
    Track {
        id: id.clone(),
        title: "未知曲目".into(),
        artists: Vec::new(),
        album: None,
        album_id: None,
        album_audio_id: None,
        duration_ms: None,
        cover_ref: None,
        qualities: Vec::new(),
        extra_hashes: BTreeMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证权限接口多档品质（含 Hi-Res）识别与无权限过滤。
    #[test]
    fn tst_prvqlt() {
        let root = crate::deps::serde_json::json!({
            "data": [{
                "hash": "a1", "quality": "128", "level": 1,
                "relate_goods": [
                    {"hash": "a2", "quality": "320", "level": 1},
                    {"hash": "a3", "quality": "flac", "level": 1},
                    {"hash": "a4", "quality": "high", "level": 0}
                ]
            }]
        });
        let avail = prsawqlts(&root);
        assert!(avail.contains(&AudioQuality::Standard));
        assert!(avail.contains(&AudioQuality::High));
        assert!(avail.contains(&AudioQuality::Lossless));
        assert!(!avail.contains(&AudioQuality::HiRes));
    }
}
