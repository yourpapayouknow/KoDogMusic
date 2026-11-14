// 账号收藏与歌单读写。
use crate::application::ports::LibraryPort;
use crate::domain::error::{AppError, AuthError, CatalogError, NetworkError};
use crate::domain::ids::{ProviderId, TrackId};
use crate::domain::track::{clnttl, TrackSummary, UserPlaylistMeta};
use crate::provider::kugou::client::KugouClient;
use crate::deps::serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

#[derive(Default)]
struct Remote {
    userid: u64,
    listid: String,
    files: HashMap<i64, HashMap<String, u64>>,
}

pub struct KugouLibrary {
    client: KugouClient,
    remote: Mutex<Remote>,
}

impl KugouLibrary {
    /// 共享官方账号会话。
    pub fn nw(client: KugouClient) -> Self {
        Self { client, remote: Mutex::new(Remote::default()) }
    }

    /// 分页拉取账号下全部原始歌单节点。
    fn rawpls(&self) -> Result<(u64, Vec<Value>), AppError> {
        let session = self.client.sess();
        if session.user_id == 0 || session.token.is_empty() {
            return Err(AuthError::Missing.into());
        }
        let mut params = self.client.params();
        params.insert("plat".into(), "1".into());
        let mut page = 1;
        let mut lists = Vec::new();
        loop {
            let body = json!({
                "userid": session.user_id.to_string(),
                "token": session.token,
                "total_ver": 979,
                "type": 2,
                "page": page.to_string(),
                "pagesize": "100"
            });
            let root = self.client.req("https://gateway.kugou.com/v7/get_all_list", params.clone(), Some(&body), false, "cloudlist.service.kugou.com")?;
            let items = root.get("data").and_then(|data| data.get("info")).and_then(Value::as_array)
                .ok_or_else(|| NetworkError::Server("账号歌单响应格式异常".into()))?;
            lists.extend(items.iter().cloned());
            if items.len() < 100 || page >= 100 { break; }
            page += 1;
        }
        Ok((session.user_id, lists))
    }

    /// 查询唯一的账号音乐收藏歌单。
    fn likeid(&self) -> Result<String, AppError> {
        let (_, lists) = self.rawpls()?;
        pckfvrt(&lists)
    }
}

impl LibraryPort for KugouLibrary {
    /// 分页读取账号收藏并保存用于删除的真实资源ID。
    fn fvrts(&self) -> Result<Vec<TrackSummary>, AppError> {
        let listid = self.likeid()?;
        let lid: i64 = listid.parse().map_err(|_| NetworkError::Server("收藏歌单ID无效".into()))?;
        let tracks = self.pltrks(lid)?;
        let session = self.client.sess();
        let mut remote = self.remote.lock().unwrap();
        remote.userid = session.user_id;
        remote.listid = listid;
        Ok(tracks)
    }

    /// 向正确歌单提交一次变更，不重试写入。
    fn tglfvrt(&self, track: &TrackSummary) -> Result<bool, AppError> {
        if track.id.provider != ProviderId::Kugou {
            return Err(CatalogError::InvalidData("本地文件不能同步到账号收藏".into()).into());
        }
        let tracks = self.fvrts()?;
        let remove = tracks.iter().any(|item| item.id == track.id);
        let lid: i64 = {
            let remote = self.remote.lock().unwrap();
            remote.listid.parse().map_err(|_| NetworkError::Server("收藏歌单ID无效".into()))?
        };
        if remove {
            self.deltrk(lid, &track.id)?;
        } else {
            self.addtrks(lid, std::slice::from_ref(track))?;
        }
        Ok(!remove)
    }

    /// 从官方列表确认收藏状态。
    fn isfvrt(&self, id: &TrackId) -> Result<bool, AppError> {
        Ok(self.fvrts()?.iter().any(|track| &track.id == id))
    }

    /// 读取账号下所有自建与收藏歌单摘要。
    fn usrpls(&self) -> Result<Vec<UserPlaylistMeta>, AppError> {
        let (userid, items) = self.rawpls()?;
        let like_id = pckfvrt(&items).ok();
        let mut out = Vec::new();
        for item in &items {
            if fldnum(item, &["is_del"]) == Some(1) { continue; }
            let Some(id) = fldstr(item, &["listid", "list_id"]) else { continue; };
            let Some(name) = fldstr(item, &["name", "listname"]) else { continue; };
            let list_id = id.parse::<i64>().ok();
            let global_id = fldstr(item, &["list_create_gid", "global_collection_id"]);
            let cover_key = fldstr(item, &["pic", "imgurl", "img", "cover"]).map(|u| u.replace("{size}", "480"));
            let song_count = fldnum(item, &["count", "songcount", "song_count"]).unwrap_or(0) as u32;
            let creator_id = fldnum(item, &["list_create_userid"]);
            let is_like = like_id.as_deref() == Some(id.as_str());
            let is_own = is_like
                || fldnum(item, &["is_self"]) == Some(1)
                || creator_id.is_none_or(|cid| cid == 0 || cid == userid);
            out.push(UserPlaylistMeta { id, list_id, global_id, name, cover_key, song_count, is_own, is_like });
        }
        out.sort_by_key(|p| (!p.is_like, !p.is_own));
        Ok(out)
    }

    /// 分页读取指定用户歌单全部曲目并缓存 fileid。
    fn pltrks(&self, lid: i64) -> Result<Vec<TrackSummary>, AppError> {
        let session = self.client.sess();
        if session.user_id == 0 || session.token.is_empty() {
            return Err(AuthError::Missing.into());
        }
        let listid = lid.to_string();
        let mut tracks = Vec::new();
        let mut files = HashMap::new();
        let mut page = 1;
        loop {
            let body = json!({
                "listid": listid,
                "userid": session.user_id.to_string(),
                "area_code": 1,
                "show_relate_goods": 0,
                "pagesize": "100",
                "allplatform": 1,
                "show_cover": 1,
                "type": 0,
                "token": session.token,
                "page": page.to_string()
            });
            let root = self.client.req("https://gateway.kugou.com/v4/get_list_all_file", self.client.params(), Some(&body), false, "cloudlist.service.kugou.com")?;
            let data = root.get("data").ok_or_else(|| NetworkError::Server("账号歌单缺少数据".into()))?;
            let count = fldnum(data, &["count"]).unwrap_or(0);
            let rows = match data.get("info") {
                Some(Value::Array(rows)) => rows.as_slice(),
                Some(Value::Null) | None if count == 0 => &[],
                _ => return Err(NetworkError::Server("账号歌单响应格式异常".into()).into()),
            };
            for row in rows {
                let (track, fileid) = rowtrk(row)?;
                if let Some(fileid) = fileid {
                    files.entry(track.id.raw_id.clone()).or_insert(fileid);
                }
                tracks.push(track);
            }
            if tracks.len() as u64 >= count || rows.is_empty() { break; }
            page += 1;
        }
        if self.client.sess().user_id != session.user_id { return Err(AuthError::Expired.into()); }
        let mut remote = self.remote.lock().unwrap();
        remote.userid = session.user_id;
        remote.files.insert(lid, files);
        Ok(tracks)
    }

    /// 在云端创建用户自建歌单并返回歌单元数据。
    fn mkpl(&self, name: &str) -> Result<UserPlaylistMeta, AppError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(CatalogError::InvalidData("歌单名称不能为空".into()).into());
        }
        let root = self.client.mkpl(name)?;
        let data = root.get("data").unwrap_or(&root);
        let list_id = fldnum(data, &["listid", "list_id", "id"])
            .or_else(|| data.get("info").and_then(|info| fldnum(info, &["listid", "list_id", "id"])))
            .and_then(|v| i64::try_from(v).ok())
            .ok_or_else(|| NetworkError::Server("新建歌单未返回listid".into()))?;
        Ok(UserPlaylistMeta {
            id: list_id.to_string(),
            list_id: Some(list_id),
            global_id: None,
            name: name.to_string(),
            cover_key: None,
            song_count: 0,
            is_own: true,
            is_like: false,
        })
    }

    /// 分批将曲目列表追加至云端歌单。
    fn addtrks(&self, lid: i64, tracks: &[TrackSummary]) -> Result<usize, AppError> {
        if tracks.is_empty() { return Ok(0); }
        let mut written = 0usize;
        for chunk in tracks.chunks(20) {
            let entries: Vec<Value> = chunk.iter().map(|track| {
                if track.id.provider != ProviderId::Kugou {
                    return Value::Null;
                }
                let raw = track.colttl();
                let name = raw.replace(['|', ','], " ");
                let album_id = track.album_id.as_deref().and_then(|s| s.parse::<i64>().ok()).unwrap_or(0);
                let mixsongid = track.album_audio_id.unwrap_or(0);
                json!({
                    "number": 1,
                    "name": name,
                    "hash": track.id.raw_id,
                    "size": 0,
                    "sort": 0,
                    "timelen": 0,
                    "bitrate": 0,
                    "album_id": album_id,
                    "mixsongid": mixsongid
                })
            }).filter(|v| !v.is_null()).collect();
            if entries.is_empty() { continue; }
            self.client.addpltrks(lid, &entries)?;
            written += entries.len();
        }
        if written == 0 {
            return Err(CatalogError::InvalidData("本地文件不能同步到账号歌单".into()).into());
        }
        Ok(written)
    }

    /// 从云端歌单移除指定曲目。
    fn deltrk(&self, lid: i64, tid: &TrackId) -> Result<(), AppError> {
        if tid.provider != ProviderId::Kugou {
            return Err(CatalogError::InvalidData("本地文件不能从云歌单移除".into()).into());
        }
        let session = self.client.sess();
        let cached_id = {
            let remote = self.remote.lock().unwrap();
            if remote.userid == session.user_id {
                remote.files.get(&lid).and_then(|m| m.get(&tid.raw_id)).copied()
            } else {
                None
            }
        };
        let fileid = match cached_id {
            Some(id) => id,
            None => {
                let _ = self.pltrks(lid)?;
                let remote = self.remote.lock().unwrap();
                remote.files.get(&lid).and_then(|m| m.get(&tid.raw_id)).copied()
                    .ok_or_else(|| NetworkError::Server("歌单中未找到该曲目删除标识".into()))?
            }
        };
        let body = json!({
            "listid": lid.to_string(),
            "userid": session.user_id.to_string(),
            "data": [{"fileid": fileid}],
            "type": 0,
            "token": session.token,
            "list_ver": 0
        });
        self.client.req("https://gateway.kugou.com/v4/delete_songs", self.client.params(), Some(&body), false, "cloudlist.service.kugou.com")?;
        Ok(())
    }

    /// 删除云端自建歌单并清除本地缓存。
    fn dellst(&self, lid: i64) -> Result<(), AppError> {
        self.client.dellst(lid)?;
        let mut remote = self.remote.lock().unwrap();
        if remote.listid == lid.to_string() { remote.listid.clear(); }
        remote.files.remove(&lid);
        Ok(())
    }
}

/// 从候选中优先选择明确的我喜欢歌单。
pub(super) fn pckfvrt(items: &[Value]) -> Result<String, AppError> {
    for name in ["我喜欢的音乐", "我喜欢", "默认收藏"] {
        let candidates = items.iter().filter(|item| fldstr(item, &["name"]).as_deref().is_some_and(|label| label.trim() == name) && fldnum(item, &["is_del"]) != Some(1)).collect::<Vec<_>>();
        if candidates.len() > 1 { return Err(NetworkError::Server("账号存在多个同名收藏歌单".into()).into()); }
        if let Some(item) = candidates.first() { return fldstr(item, &["listid"]).ok_or_else(|| NetworkError::Server("收藏歌单缺少标识".into()).into()); }
    }
    Err(NetworkError::Server("无法明确识别账号收藏歌单".into()) .into())
}

/// 读取字符串或数字形式的协议字段。
pub(super) fn fldstr(value: &Value, names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| value.get(*name).and_then(|value| value.as_str().map(str::trim).filter(|value| !value.is_empty()).map(ToString::to_string).or_else(|| value.as_u64().map(|value| value.to_string()))))
}

/// 读取整数形式的协议字段。
pub(super) fn fldnum(value: &Value, names: &[&str]) -> Option<u64> {
    names.iter().find_map(|name| value.get(*name).and_then(|value| value.as_u64().or_else(|| value.as_str().and_then(|value| value.trim().parse().ok()))))
}

/// 解析云端、专辑及日推歌曲字段并保留扩展元数据与删除 ID。
pub(super) fn rowtrk(row: &Value) -> Result<(TrackSummary, Option<u64>), AppError> {
    let audio_info = row.get("audio_info");
    let hash = fldstr(row, &["hash", "FileHash", "file_hash"])
        .or_else(|| audio_info.and_then(|info| fldstr(info, &["hash", "hash_128", "hash_320", "hash_flac", "hash_high"])))
        .ok_or(CatalogError::NotFound)?;
    let name = fldstr(row, &["OriSongName", "ori_audio_name", "SongName", "songname", "audio_name", "FileName", "filename", "name"])
        .or_else(|| row.get("base").and_then(|b| fldstr(b, &["audio_name", "songname"])))
        .map(|s| clnttl(&s))
        .ok_or(CatalogError::NotFound)?;
    let mut artist = fldstr(row, &["SingerName", "singername", "author_name"])
        .or_else(|| row.get("base").and_then(|b| fldstr(b, &["author_name", "singername"])))
        .unwrap_or_default();
    let mut title = name.clone();
    if let Some((prefix, song)) = name.split_once(" - ") {
        if artist.is_empty() || artist == prefix { artist = prefix.to_string(); title = clnttl(song); }
    }
    let cover = fldstr(row, &["sizable_cover", "Image", "img", "cover", "album_image"])
        .or_else(|| row.get("trans_param").and_then(|value| fldstr(value, &["union_cover", "cover"])))
        .or_else(|| row.get("album_info").and_then(|value| fldstr(value, &["sizable_cover", "cover"])))
        .or_else(|| row.get("albuminfo").and_then(|value| fldstr(value, &["cover", "sizable_cover"])));
    let raw_dur = fldnum(row, &["timelength", "time_length", "timelen", "Duration", "duration"])
        .or_else(|| audio_info.and_then(|info| fldnum(info, &["duration", "duration_128", "duration_320", "duration_flac"])));
    let duration = raw_dur.map(|v| if v == 0 { 0 } else if v >= 10_000 { v } else { v.saturating_mul(1000) });
    let album_id = fldstr(row, &["album_id", "AlbumID"])
        .or_else(|| row.get("album_info").and_then(|v| fldstr(v, &["album_id", "id"])))
        .or_else(|| row.get("albuminfo").and_then(|v| fldstr(v, &["id", "album_id"])))
        .filter(|s| s != "0");
    let album_name = fldstr(row, &["AlbumName", "album_name"])
        .or_else(|| row.get("album_info").and_then(|v| fldstr(v, &["album_name", "name"])))
        .or_else(|| row.get("albuminfo").and_then(|v| fldstr(v, &["name", "album_name"])));
    let album_audio_id = fldnum(row, &["AlbumAudioID", "album_audio_id", "MixSongID", "mixsongid"]).and_then(|v| i64::try_from(v).ok());
    let mut extra_hashes = BTreeMap::new();
    if let Some(info) = audio_info {
        for (q, k) in [("128", "hash_128"), ("320", "hash_320"), ("flac", "hash_flac"), ("high", "hash_high")] {
            if let Some(h) = fldstr(info, &[k]) {
                extra_hashes.insert(q.to_string(), h.to_ascii_lowercase());
            }
        }
    }
    for (q, k) in [("320", "hash_320"), ("flac", "hash_flac"), ("high", "hash_high")] {
        if let Some(h) = fldstr(row, &[k]) {
            extra_hashes.entry(q.to_string()).or_insert_with(|| h.to_ascii_lowercase());
        }
    }
    Ok((
        TrackSummary {
            id: TrackId::kg(hash),
            title,
            artist_text: artist,
            cover_key: cover.map(|url| url.replace("{size}", "480")),
            duration_ms: duration,
            album_id,
            album_name,
            album_audio_id,
            extra_hashes,
        },
        fldnum(row, &["Fileid", "FileId", "fileid", "file_id"]),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证歌单优先级、同名歧义、日推/专辑格式及歌曲元数据。
    #[test]
    fn tst_cldparse() {
        let lists = [json!({"name":"默认收藏","listid":1}), json!({"name":"我喜欢","listid":2})];
        assert_eq!(pckfvrt(&lists).unwrap(), "2");
        assert!(pckfvrt(&[lists[1].clone(), lists[1].clone()]).is_err());
        let (track, fileid) = rowtrk(&json!({"hash":"AbC","name":"歌手 - 歌曲.mp3","timelen":123000,"fileid":7,"trans_param":{"union_cover":"https://example.com/{size}/cover.png"}})).unwrap();
        assert_eq!(track.id.raw_id, "abc");
        assert_eq!(track.title, "歌曲");
        assert_eq!(track.artist_text, "歌手");
        assert_eq!(track.duration_ms, Some(123000));
        assert_eq!(fileid, Some(7));
        let (daily, _) = rowtrk(&json!({
            "hash":"DEF","ori_audio_name":"迟来的人","author_name":"蔡琴","time_length":245,
            "sizable_cover":"https://example.com/{size}/d.jpg","album_id":"888","album_name":"你不要那样看着我的眼睛",
            "audio_info":{"hash_320":"H320","hash_flac":"HFLAC","hash_high":"HHIRES"}
        })).unwrap();
        assert_eq!(daily.title, "迟来的人");
        assert_eq!(daily.album_id.as_deref(), Some("888"));
        assert_eq!(daily.album_name.as_deref(), Some("你不要那样看着我的眼睛"));
        assert_eq!(daily.duration_ms, Some(245000));
        assert_eq!(daily.extra_hashes.get("high").map(String::as_str), Some("hhires"));
        assert!(rowtrk(&json!({"name":"缺少标识"})).is_err());
    }

    /// 只读检查实际账号收藏，不修改云端数据。
    #[test]
    #[ignore = "需要用户已经扫码登录"]
    fn tst_cldnet() {
        let path = std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap()).join("KoDogLite/secrets.dat");
        let library = KugouLibrary::nw(KugouClient::opn(path).unwrap());
        let tracks = library.fvrts().unwrap();
        assert!(!library.remote.lock().unwrap().listid.is_empty());
        eprintln!("verified cloud favorite count: {}", tracks.len());
    }

    /// 仅在用户明确授权后验证指定歌曲的云端增删往返。
    #[test]
    #[ignore = "会修改真实账号收藏，必须先取得用户授权"]
    fn tst_cldrw() {
        assert_eq!(std::env::var("KODOG_CLOUD_RW").as_deref(), Ok("qingtian-roundtrip"), "需要先取得用户授权并开启单次测试开关");
        let path = std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap()).join("KoDogLite/secrets.dat");
        let client = KugouClient::opn(path).unwrap();
        let library = KugouLibrary::nw(client.clone());
        let id = TrackId::kg("b3a52a7a958bf0aed0ebfba2e9a818b7");
        assert!(!library.isfvrt(&id).unwrap(), "歌曲已在原有收藏中，停止测试");
        let catalog = crate::provider::kugou::catalog::KugouCatalog::nw(client);
        let tracks = crate::application::ports::CatalogPort::srchtrks(&catalog, "晴天", 1, 30).unwrap();
        let track = tracks.iter().find(|track| track.id == id).expect("搜索中未找到已确认的测试歌曲");
        assert!(library.tglfvrt(track).unwrap());
        waitfvrt(&library, &id, true);
        assert!(!library.tglfvrt(track).unwrap());
        waitfvrt(&library, &id, false);
    }

    /// 等待服务器读取结果可见，不重试写入。
    fn waitfvrt(library: &KugouLibrary, id: &TrackId, expected: bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if library.isfvrt(id).unwrap() == expected { return; }
            assert!(std::time::Instant::now() < deadline, "云端变更尚未可见");
            std::thread::sleep(std::time::Duration::from_millis(400));
        }
    }

    /// 创建临时歌单并立即删除，验证删除歌单接口往返（自建自删，不触碰既有歌单）。
    #[test]
    #[ignore = "会在真实账号上创建并删除一个临时歌单"]
    fn tst_dellst() {
        let path = std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap()).join("KoDogLite/secrets.dat");
        let library = KugouLibrary::nw(KugouClient::opn(path).unwrap());
        let created = library.mkpl("KoDog临时验证-可删").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1200));
        library.dellst(created.list_id.unwrap()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1200));
        let names: Vec<String> = library.usrpls().unwrap().into_iter().map(|p| p.name).collect();
        assert!(!names.iter().any(|name| name == "KoDog临时验证-可删"), "临时歌单仍在账号中");
        println!("临时歌单已创建并删除成功");
    }

}
