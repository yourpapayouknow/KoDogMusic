use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use crate::application::ports::{AccountPort, CatalogPort, LibraryPort, StreamPort};
use crate::domain::account::{AccountState, CheckInState};
use crate::domain::error::{AppError, CatalogError};
use crate::domain::ids::{ProviderId, TrackId};
use crate::domain::track::{AudioQuality, LyricLine, ResolvedSource, Track, TrackSummary, UserPlaylistMeta};

/// 模拟 Provider 适配器
pub struct MockProvider {
    pub is_checked: AtomicBool,
    playlists: Mutex<Vec<UserPlaylistMeta>>,
    playlist_tracks: Mutex<HashMap<i64, Vec<TrackSummary>>>,
    next_list_id: Mutex<i64>,
}

impl MockProvider {
    /// 构造模拟适配器
    pub fn nw() -> Self {
        let like = UserPlaylistMeta {
            id: "1".into(),
            list_id: Some(1),
            global_id: None,
            name: "我喜欢".into(),
            cover_key: None,
            song_count: 2,
            is_own: true,
            is_like: true,
        };
        let fav1 = TrackSummary {
            id: TrackId::kg("fav_1"),
            title: "星晴".to_string(),
            artist_text: "周杰伦".to_string(),
            cover_key: None,
            duration_ms: Some(255_000),
            album_id: Some("fav_alb_1".into()),
            album_name: Some("Jay".into()),
            album_audio_id: None,
            extra_hashes: BTreeMap::new(),
        };
        let fav2 = TrackSummary {
            id: TrackId::kg("fav_2"),
            title: "晴天".to_string(),
            artist_text: "周杰伦".to_string(),
            cover_key: None,
            duration_ms: Some(269_000),
            album_id: Some("966846".into()),
            album_name: Some("叶惠美".into()),
            album_audio_id: None,
            extra_hashes: BTreeMap::new(),
        };
        let mut tracks = HashMap::new();
        tracks.insert(1, vec![fav1, fav2]);
        Self {
            is_checked: AtomicBool::new(false),
            playlists: Mutex::new(vec![like]),
            playlist_tracks: Mutex::new(tracks),
            next_list_id: Mutex::new(100),
        }
    }
}

impl Default for MockProvider {
    /// 构造默认模拟数据源。
    fn default() -> Self {
        Self::nw()
    }
}

impl CatalogPort for MockProvider {
    /// 返回十条固定结构的模拟搜索结果。
    fn srchtrks(&self, qry: &str, _pg: u32, _lmt: u32) -> Result<Vec<TrackSummary>, AppError> {
        let mut list = Vec::new();
        for i in 1..=10 {
            list.push(TrackSummary {
                id: TrackId {
                    provider: ProviderId::Kugou,
                    raw_id: format!("mock_{}_{}", qry, i),
                },
                title: format!("{} - 曲目 {}", qry, i),
                artist_text: format!("测试艺人 {}", i),
                cover_key: None,
                duration_ms: Some(210_000),
                album_id: Some("mock_album_1".to_string()),
                album_name: Some("你不要那样看着我的眼睛".to_string()),
                album_audio_id: Some(100 + i as i64),
                extra_hashes: BTreeMap::new(),
            });
        }
        Ok(list)
    }

    /// 返回模拟曲目详情。
    fn trkdtl(&self, id: &TrackId) -> Result<Track, AppError> {
        Ok(Track {
            id: id.clone(),
            title: format!("示例曲目 {}", id.raw_id),
            artists: vec!["测试歌手".to_string()],
            album: Some("测试专辑".to_string()),
            album_id: Some("mock_album_1".to_string()),
            album_audio_id: Some(101),
            duration_ms: Some(210_000),
            cover_ref: None,
            qualities: vec![AudioQuality::Standard, AudioQuality::High],
            extra_hashes: BTreeMap::new(),
        })
    }

    /// 返回三条模拟歌词。
    fn lyrc(&self, _id: &TrackId) -> Result<Vec<LyricLine>, AppError> {
        Ok(vec![
            LyricLine { begin_ms: 0, end_ms: Some(3000), text: "前奏...".to_string() },
            LyricLine { begin_ms: 3000, end_ms: Some(6000), text: "第一句歌词".to_string() },
            LyricLine { begin_ms: 6000, end_ms: Some(9000), text: "第二句歌词".to_string() },
        ])
    }

    /// 返回两条模拟每日推荐。
    fn dailyrec(&self) -> Result<Vec<TrackSummary>, AppError> {
        let mut t1 = TrackSummary::nw(TrackId::kg("daily_1"), "大城小爱", "王力宏");
        t1.duration_ms = Some(270_000);
        let mut t2 = TrackSummary::nw(TrackId::kg("daily_2"), "后来", "刘若英");
        t2.duration_ms = Some(340_000);
        Ok(vec![t1, t2])
    }

    /// 返回至多三个模拟推荐歌单。
    fn recpl(&self, lmt: u32) -> Result<Vec<UserPlaylistMeta>, AppError> {
        Ok((0..lmt.min(3)).map(|i| UserPlaylistMeta {
            id: format!("rec_gid_{i}"),
            list_id: None,
            global_id: Some(format!("rec_gid_{i}")),
            name: format!("推荐歌单 {}", i + 1),
            cover_key: None,
            song_count: 2,
            is_own: false,
            is_like: false,
        }).collect())
    }

    /// 返回一条模拟公开歌单曲目。
    fn pubpltrks(&self, gid: &str) -> Result<Vec<TrackSummary>, AppError> {
        let t = TrackSummary::nw(TrackId::kg(format!("{gid}_1")), format!("{gid} 首曲"), "测试艺人");
        Ok(vec![t])
    }

    /// 返回模拟专辑及曲目。
    fn albdtl(&self, aid: &str) -> Result<(String, Option<String>, Vec<TrackSummary>), AppError> {
        if aid != "mock_album_1" && aid != "966846" {
            return Err(CatalogError::NotFound.into());
        }
        let mut t1 = TrackSummary::nw(TrackId::kg("alb_1"), "雨中城市", "蔡琴");
        t1.album_id = Some(aid.into());
        t1.album_name = Some("你不要那样看着我的眼睛".into());
        let mut t2 = TrackSummary::nw(TrackId::kg("alb_2"), "迟来的人", "蔡琴");
        t2.album_id = Some(aid.into());
        t2.album_name = Some("你不要那样看着我的眼睛".into());
        let mut t3 = TrackSummary::nw(TrackId::kg("alb_3"), "你不要那样看着我的眼睛", "蔡琴");
        t3.album_id = Some(aid.into());
        t3.album_name = Some("你不要那样看着我的眼睛".into());
        Ok(("你不要那样看着我的眼睛".into(), None, vec![t1, t2, t3]))
    }

    /// 返回固定的三档模拟音质。
    fn prvqlt(&self, _trks: &[TrackSummary]) -> Result<Vec<AudioQuality>, AppError> {
        Ok(vec![AudioQuality::High, AudioQuality::Lossless, AudioQuality::HiRes])
    }
}

impl AccountPort for MockProvider {
    /// 返回固定的已登录用户。
    fn stt(&self) -> AccountState {
        AccountState::Authorized {
            user_id: "10001".to_string(),
            display_name: "测试用户".to_string(),
            avatar_ref: None,
            vip_type: 1,
        }
    }

    /// 模拟签到状态无需同步。
    fn synck(&self) {}

    /// 返回内存中的签到状态。
    fn chkstt(&self) -> CheckInState {
        if self.is_checked.load(Ordering::SeqCst) {
            CheckInState::Checked
        } else {
            CheckInState::Available
        }
    }

    /// 标记为已签到并返回结果。
    fn dochk(&self) -> Result<CheckInState, AppError> {
        self.is_checked.store(true, Ordering::SeqCst);
        Ok(CheckInState::Checked)
    }

    /// 模拟登出。
    fn lgout(&self) -> Result<(), AppError> {
        Ok(())
    }
}

impl LibraryPort for MockProvider {
    /// 返回内存中的收藏曲目。
    fn fvrts(&self) -> Result<Vec<TrackSummary>, AppError> {
        Ok(self.playlist_tracks.lock().unwrap().get(&1).cloned().unwrap_or_default())
    }

    /// 切换内存收藏状态。
    fn tglfvrt(&self, track: &TrackSummary) -> Result<bool, AppError> {
        let mut map = self.playlist_tracks.lock().unwrap();
        let list = map.entry(1).or_default();
        if let Some(pos) = list.iter().position(|t| t.id == track.id) {
            list.remove(pos);
            Ok(false)
        } else {
            list.push(track.clone());
            Ok(true)
        }
    }

    /// 查询内存收藏状态。
    fn isfvrt(&self, id: &TrackId) -> Result<bool, AppError> {
        Ok(self.playlist_tracks.lock().unwrap().get(&1).is_some_and(|l| l.iter().any(|t| &t.id == id)))
    }

    /// 汇总内存歌单及曲目数。
    fn usrpls(&self) -> Result<Vec<UserPlaylistMeta>, AppError> {
        let counts = self.playlist_tracks.lock().unwrap();
        let mut pls = self.playlists.lock().unwrap().clone();
        for p in &mut pls {
            if let Some(lid) = p.list_id {
                p.song_count = counts.get(&lid).map_or(0, Vec::len) as u32;
            }
        }
        Ok(pls)
    }

    /// 返回指定内存歌单曲目。
    fn pltrks(&self, lid: i64) -> Result<Vec<TrackSummary>, AppError> {
        Ok(self.playlist_tracks.lock().unwrap().get(&lid).cloned().unwrap_or_default())
    }

    /// 创建内存歌单。
    fn mkpl(&self, nm: &str) -> Result<UserPlaylistMeta, AppError> {
        let mut next = self.next_list_id.lock().unwrap();
        let lid = *next;
        *next += 1;
        let meta = UserPlaylistMeta {
            id: lid.to_string(),
            list_id: Some(lid),
            global_id: None,
            name: nm.to_string(),
            cover_key: None,
            song_count: 0,
            is_own: true,
            is_like: false,
        };
        self.playlists.lock().unwrap().push(meta.clone());
        self.playlist_tracks.lock().unwrap().insert(lid, Vec::new());
        Ok(meta)
    }

    /// 追加曲目到内存歌单。
    fn addtrks(&self, lid: i64, trks: &[TrackSummary]) -> Result<usize, AppError> {
        let mut map = self.playlist_tracks.lock().unwrap();
        let list = map.entry(lid).or_default();
        list.extend(trks.iter().cloned());
        Ok(trks.len())
    }

    /// 从内存歌单移除曲目。
    fn deltrk(&self, lid: i64, tid: &TrackId) -> Result<(), AppError> {
        let mut map = self.playlist_tracks.lock().unwrap();
        if let Some(list) = map.get_mut(&lid) {
            list.retain(|t| &t.id != tid);
        }
        Ok(())
    }
}

impl StreamPort for MockProvider {
    /// 返回固定的模拟流地址。
    fn rslv(&self, id: &TrackId, _qlt: AudioQuality) -> Result<ResolvedSource, AppError> {
        Ok(ResolvedSource {
            url_or_local: format!("https://mock.stream/{}.mp3", id.raw_id),
            codec_hint: Some("mp3".to_string()),
        })
    }
}
