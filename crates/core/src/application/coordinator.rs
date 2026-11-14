use std::sync::Arc;
use crate::application::commands::{AppCommand, TargetScope};
use crate::application::ports::{AccountPort, CatalogPort, LibraryPort, PlaybackPort, StoragePort, StreamPort};
use crate::application::snapshot::{AddToPrompt, AlbumView, DeckSummary, DownloadPrompt, OpenedPlaylist, UiSnapshot};
use crate::domain::account::{AccountState, QrStage};
use crate::domain::error::{AppError, CatalogError, StorageError};
use crate::domain::ids::TrackId;
use crate::domain::playback::{PlaybackState, RepeatMode};
use crate::domain::queue::Queue;
use crate::domain::settings::AppSettings;
use crate::domain::track::{CoverDeck, DeckKind, LyricLine, ResolvedSource, TrackSummary, UserPlaylistMeta};

/// 应用全局用例协调器。
pub struct Coordinator {
    pub catalog: Arc<dyn CatalogPort>,
    pub account: Arc<dyn AccountPort>,
    pub library: Arc<dyn LibraryPort>,
    pub stream: Arc<dyn StreamPort>,
    pub playback: Arc<dyn PlaybackPort>,
    pub storage: Arc<dyn StoragePort>,
    pub queue: Queue,
    pub settings: AppSettings,
    pub snapshot: UiSnapshot,
    decks: Vec<CoverDeck>,
    active_deck: Option<usize>,
    user_playlists: Vec<UserPlaylistMeta>,
    opened_playlist: Option<OpenedPlaylist>,
    add_to_prompt: Option<AddToPrompt>,
    confirm_unlike: Option<(String, TrackSummary)>,
    download_prompt: Option<DownloadPrompt>,
    queue_version: u64,
    settings_ready: bool,
    end_handled: bool,
    lyrics: Vec<LyricLine>,
    download: Option<crate::services::downloads::DownloadTask>,
}

impl Coordinator {
    /// 恢复队列、设置与本地收藏。
    pub fn nw(
        catalog: Arc<dyn CatalogPort>, account: Arc<dyn AccountPort>,
        library: Arc<dyn LibraryPort>, stream: Arc<dyn StreamPort>,
        playback: Arc<dyn PlaybackPort>, storage: Arc<dyn StoragePort>,
    ) -> Self {
        let loaded = storage.ldcfg();
        let settings_ready = loaded.is_ok();
        let mut error = loaded.as_ref().err().map(ToString::to_string);
        let settings = loaded.unwrap_or_default();
        let mut queue = Queue::nw();
        match storage.ldqu() {
            Ok((items, cursor)) => {
                for item in items { queue.psh(item); }
                if let Some(index) = cursor { queue.stcrr(index); }
            }
            Err(e) => error = Some(e.to_string()),
        }
        queue.repeat_mode = settings.audio.repeat;
        queue.shuffle = settings.audio.shuffle;
        playback.stvlm(settings.audio.volume);
        let favorites = match storage.ldfvrt() {
            Ok(tracks) => tracks,
            Err(e) => { error = Some(e.to_string()); Vec::new() }
        };
        let mut coord = Self {
            catalog, account, library, stream, playback, storage, queue, settings,
            snapshot: UiSnapshot { favorites, favorites_local: true, error, ..UiSnapshot::default() },
            decks: Vec::new(),
            active_deck: None,
            user_playlists: Vec::new(),
            opened_playlist: None,
            add_to_prompt: None,
            confirm_unlike: None,
            download_prompt: None,
            queue_version: u64::MAX, settings_ready, end_handled: false, lyrics: Vec::new(), download: None,
        };
        if let Some(id) = coord.queue.crr() { coord.snapshot.current = coord.storage.ldtrk(id).ok().flatten(); }
        coord.updssht();
        coord
    }

    /// 执行命令并保留可呈现的错误。
    pub fn hndlcmd(&mut self, cmd: AppCommand) {
        self.snapshot.error = None;
        if let Err(error) = self.exec(cmd) { self.snapshot.error = Some(error.to_string()); }
        self.updssht();
    }

    /// 分派用例并保存已变更状态。
    fn exec(&mut self, cmd: AppCommand) -> Result<(), AppError> {
        let persist = matches!(&cmd,
            AppCommand::SetVolume(_) | AppCommand::SetRepeat(_) | AppCommand::ToggleShuffle |
            AppCommand::SetTheme(_) | AppCommand::SetQuality(_) | AppCommand::SetClose(_) |
            AppCommand::SetLyrics(_) | AppCommand::SetCacheDir(_) | AppCommand::SetDownloadDir(_));
        if persist && !self.settings_ready { return Err(StorageError::Corrupt.into()); }
        let cache_directory = matches!(&cmd, AppCommand::SetCacheDir(_));
        match cmd {
            AppCommand::Nav(route) => {
                self.snapshot.route = route;
                // 离页关闭专辑视图。
                self.snapshot.album_view = None;
            }
            AppCommand::AuthLogin => self.snapshot.login_qr = Some(self.account.qrstrt()?),
            AppCommand::AuthCancel => self.snapshot.login_qr = None,
            AppCommand::AuthPoll => {
                if let Some(qr) = &mut self.snapshot.login_qr {
                    qr.stage = self.account.qrpoll(&qr.key)?;
                    if qr.stage == QrStage::Authorized {
                        self.snapshot.login_qr = None;
                        self.lddecks(true)?;
                    }
                }
            }
            AppCommand::AuthLogout => {
                self.account.lgout()?;
                self.playback.stp();
                self.snapshot.login_qr = None;
                self.snapshot.current = None;
                self.decks.clear();
                self.active_deck = None;
                self.user_playlists.clear();
                self.opened_playlist = None;
                self.add_to_prompt = None;
                self.confirm_unlike = None;
                self.download_prompt = None;
                self.queue.clr();
                self.snapshot.favorites = self.storage.ldfvrt()?;
                self.snapshot.favorites_local = true;
            }
            AppCommand::CheckIn => { self.account.dochk()?; }
            AppCommand::LoadCoverFlow => self.lddecks(false)?,
            AppCommand::SelectDeck { index, auto_play } => self.seldeck(index, auto_play)?,
            AppCommand::RemoveDeck(index) => self.rmvdeck(index)?,
            AppCommand::Search(query) => {
                let query = query.trim();
                if query.chars().count() > 256 { return Err(CatalogError::InvalidData("搜索内容过长".into()).into()); }
                self.snapshot.album_view = None;
                self.snapshot.search_results.clear();
                if !query.is_empty() {
                    let tracks = self.catalog.srchtrks(query, 1, 30)?;
                    self.storage.svtrks(&tracks)?;
                    self.snapshot.search_results = tracks;
                }
                self.queue_version = u64::MAX;
            }
            AppCommand::OpenAddTo { search_index, whole_album } => self.opnadd(search_index, whole_album)?,
            AppCommand::CancelAddTo => self.add_to_prompt = None,
            AppCommand::CreatePlaylist(name) => self.crtpl(&name)?,
            AppCommand::ConfirmAddTo { to_coverflow, to_like, playlist_ids } => {
                self.aplyadd(to_coverflow, to_like, &playlist_ids)?;
            }
            AppCommand::OpenPlaylist(id) => {
                self.snapshot.album_view = None;
                self.opnpl(&id)?;
            }
            AppCommand::ClosePlaylist => {
                self.opened_playlist = None;
                self.snapshot.album_view = None;
            }
            AppCommand::PlayPlaylist(id) => self.plypl(&id, true)?,
            AppCommand::AddPlaylistToFlow(id) => self.plypl(&id, false)?,
            AppCommand::PlayTrack(id) => self.pltrk(id)?,
            AppCommand::QueuePlay(index) => {
                let id = self.queue.items.get(index).cloned().ok_or(CatalogError::NotFound)?;
                let previous = self.queue.cursor;
                self.queue.stcrr(index);
                if let Some(d) = self.active_deck.and_then(|i| self.decks.get_mut(i)) {
                    d.cursor = Some(index);
                }
                if let Err(error) = self.pltrk(id) {
                    self.queue.cursor = previous;
                    if let Some(d) = self.active_deck.and_then(|i| self.decks.get_mut(i)) {
                        d.cursor = previous;
                    }
                    return Err(error);
                }
            }
            AppCommand::HeartClick => self.hrtclk()?,
            AppCommand::ConfirmUnlike(confirm) => self.unlkcf(confirm)?,
            AppCommand::Download(id) => {
                if self.download.as_ref().is_some_and(|task| !task.done()) {
                    return Err(CatalogError::InvalidData("已有下载正在进行".into()).into());
                }
                let track = self.trksmry(&id)?;
                let col = track.collname();
                self.download = Some(crate::services::downloads::DownloadTask::btch(
                    vec![track], col, self.settings.audio.quality,
                    self.settings.downloads.directory.clone(), self.stream.clone(), Some(self.storage.clone()),
                )?);
            }
            AppCommand::InspectDownload { scope, search_index, whole_album } => {
                self.inspdl(scope, search_index, whole_album)?;
            }
            AppCommand::ConfirmDownload(quality) => self.cfmdl(quality)?,
            AppCommand::CancelDownloadPrompt => self.download_prompt = None,
            AppCommand::QueueAdd(id) => {
                self.trksmry(&id)?;
                self.queue.psh(id);
                self.svqueue()?;
            }
            AppCommand::QueueRemove(index) => {
                if index >= self.queue.items.len() { return Err(CatalogError::NotFound.into()); }
                let current = self.queue.cursor == Some(index);
                let state = self.playback.prgrss().state;
                let removed = self.queue.rmv(index);
                if let Some(d) = self.active_deck.and_then(|i| self.decks.get_mut(i)) {
                    if let Some(id) = removed.as_ref() { d.tracks.retain(|t| &t.id != id); }
                    d.cursor = self.queue.cursor;
                }
                if current {
                    self.playback.stp();
                    self.snapshot.current = None;
                    self.lyrics.clear();
                    if matches!(state, PlaybackState::Playing | PlaybackState::Paused | PlaybackState::Buffering) {
                        if let Some(id) = self.queue.crr().cloned() {
                            self.pltrk(id)?;
                            if state == PlaybackState::Paused { self.playback.ps(); }
                        }
                    }
                }
                self.svqueue()?;
            }
            AppCommand::QueueClear => {
                self.playback.stp();
                self.queue.clr();
                self.snapshot.current = None;
                self.lyrics.clear();
                self.svqueue()?;
            }
            AppCommand::TogglePlay => match self.playback.prgrss().state {
                PlaybackState::Playing | PlaybackState::Buffering => self.playback.ps(),
                PlaybackState::Paused => self.playback.rsm(),
                _ => {
                    if self.queue.items.is_empty() {
                        if let Some(idx) = self.active_deck {
                            self.seldeck(idx, true)?;
                            return Ok(());
                        }
                    }
                    let id = self.queue.crr().or_else(|| self.queue.items.first()).cloned();
                    if let Some(id) = id { self.pltrk(id)?; }
                }
            },
            AppCommand::Next => self.mvtrk(true)?,
            AppCommand::Prev => self.mvtrk(false)?,
            AppCommand::Stop => self.playback.stp(),
            AppCommand::Seek(position) => {
                let duration = self.playback.prgrss().duration_ms;
                self.playback.sk(if duration == 0 { position } else { position.min(duration) });
            }
            AppCommand::SetVolume(volume) => {
                if !volume.is_finite() { return Err(CatalogError::InvalidData("音量必须为有限值".into()).into()); }
                self.settings.audio.volume = volume.clamp(0.0, 1.0);
                self.playback.stvlm(self.settings.audio.volume);
            }
            AppCommand::SetRepeat(mode) => {
                self.settings.audio.repeat = mode;
                self.queue.repeat_mode = mode;
            }
            AppCommand::ToggleShuffle => {
                self.queue.shuffle = !self.queue.shuffle;
                self.settings.audio.shuffle = self.queue.shuffle;
            }
            AppCommand::ToggleFavorite(id) => {
                let track = self.trksmry(&id)?;
                if self.snapshot.favorites.iter().any(|favorite| favorite.id == track.id) {
                    let list_name = self.user_playlists.iter().find(|p| p.is_like).map(|p| p.name.as_str()).unwrap_or("我喜欢");
                    self.confirm_unlike = Some((format!("确认从歌单“{list_name}”中删除这首歌吗？"), track));
                } else {
                    self.tglfvrt_inner(&track)?;
                }
            }
            AppCommand::RefreshFavorites => self.rfrshfvrt()?,
            AppCommand::AddPlaylistJumpPlay { search_index, playlist_id, prepend } => self.addplplay(search_index, &playlist_id, prepend)?,
            AppCommand::OpenAlbum(index) => self.viewalbm(index)?,
            AppCommand::CloseAlbum => self.snapshot.album_view = None,
            AppCommand::DownloadLyrics(id) => self.dnlyrc(&id)?,
            AppCommand::PlaylistRemoveTrack(index) => self.rmpltrk(index)?,
            AppCommand::PlaylistPlay { index } => self.plypltrk(index)?,
            AppCommand::AlbumPlay { index } => self.albmplay(index)?,
            AppCommand::DeletePlaylist(id) => self.dellst(&id)?,
            AppCommand::OpenAlbumById(aid) => self.viewalbm_id(&aid)?,
            AppCommand::SetTheme(mode) => self.settings.appearance.theme = mode,
            AppCommand::SetQuality(quality) => self.settings.audio.quality = quality,
            AppCommand::SetClose(action) => self.settings.window.close_action = action,
            AppCommand::SetLyrics(enabled) => {
                self.settings.lyrics.desktop_enabled = enabled;
                if !enabled { self.lyrics.clear(); }
            }
            AppCommand::SetCacheDir(path) | AppCommand::SetDownloadDir(path) => {
                if !path.is_absolute() { return Err(CatalogError::InvalidData("请选择绝对目录".into()).into()); }
                std::fs::create_dir_all(&path).map_err(|_| StorageError::Permission)?;
                if cache_directory { self.settings.cache.directory = path; }
                else { self.settings.downloads.directory = path; }
            }
        }
        if persist { self.storage.svcfg(&self.settings)?; }
        Ok(())
    }

    /// 组装登录后的 CoverFlow（Like + 日推 + 持久化自添 + 上游推荐）。
    fn lddecks(&mut self, from_login: bool) -> Result<(), AppError> {
        self.account.synck();
        self.rfrshfvrt()?;
        let pls = self.library.usrpls().unwrap_or_default();
        self.user_playlists = pls.clone();
        let like_meta = pls.iter().find(|p| p.is_like);
        let like_deck = CoverDeck {
            id: "like".into(),
            kind: DeckKind::Like,
            title: like_meta.map(|p| p.name.clone()).unwrap_or_else(|| "我喜欢".into()),
            cover_key: like_meta.and_then(|p| p.cover_key.clone()).or_else(|| self.snapshot.favorites.first().and_then(|t| t.cover_key.clone())),
            source_id: like_meta.and_then(|p| p.list_id.map(|id| id.to_string())).unwrap_or_else(|| "like".into()),
            removable: false,
            tracks_loaded: true,
            tracks: self.snapshot.favorites.clone(),
            cursor: Some(0),
        };
        let daily_deck = CoverDeck {
            id: "daily".into(),
            kind: DeckKind::DailyRecommend,
            title: "每日推荐".into(),
            cover_key: None,
            source_id: "daily".into(),
            removable: false,
            tracks_loaded: false,
            tracks: Vec::new(),
            cursor: None,
        };
        let mut decks = vec![like_deck, daily_deck];
        for saved in self.storage.lddecks().unwrap_or_default() {
            if saved.removable && !decks.iter().any(|d| d.id == saved.id) {
                decks.push(saved);
            }
        }
        for rec in self.catalog.recpl(8).unwrap_or_default() {
            let id = format!("pub_pl:{}", rec.id);
            if !decks.iter().any(|d| d.id == id) {
                decks.push(CoverDeck {
                    id,
                    kind: DeckKind::RecommendedPlaylist,
                    title: rec.name,
                    cover_key: rec.cover_key,
                    source_id: rec.global_id.unwrap_or(rec.id),
                    removable: false,
                    tracks_loaded: false,
                    tracks: Vec::new(),
                    cursor: None,
                });
            }
        }
        self.decks = decks;
        self.active_deck = Some(1);
        self.synqu(1)?;
        self.snapshot.deck_flow_seq = self.snapshot.deck_flow_seq.wrapping_add(1);
        if from_login {
            self.snapshot.login_flow_seq = self.snapshot.login_flow_seq.wrapping_add(1);
        }
        Ok(())
    }

    /// 按需懒加载指定 CoverFlow 卡片内的曲目列表。
    fn lddeck(&mut self, index: usize) -> Result<(), AppError> {
        let Some(deck) = self.decks.get(index).cloned() else { return Err(CatalogError::NotFound.into()); };
        if deck.tracks_loaded { return Ok(()); }
        let (tracks, cover) = match deck.kind {
            DeckKind::Like => {
                self.rfrshfvrt()?;
                (self.snapshot.favorites.clone(), None)
            }
            DeckKind::DailyRecommend => (self.catalog.dailyrec()?, None),
            DeckKind::Album => {
                let (_, cov, trks) = self.catalog.albdtl(&deck.source_id)?;
                (trks, cov)
            }
            DeckKind::UserPlaylist => {
                let lid: i64 = deck.source_id.parse().map_err(|_| CatalogError::NotFound)?;
                (self.library.pltrks(lid)?, None)
            }
            DeckKind::RecommendedPlaylist => (self.catalog.pubpltrks(&deck.source_id)?, None),
            DeckKind::Single => (deck.tracks.clone(), deck.cover_key.clone()),
        };
        self.storage.svtrks(&tracks)?;
        if let Some(d) = self.decks.get_mut(index) {
            d.tracks_loaded = true;
            if d.cover_key.is_none() {
                d.cover_key = cover.or_else(|| tracks.first().and_then(|t| t.cover_key.clone()));
            }
            d.tracks = tracks;
            if d.removable {
                self.svdckst()?;
            }
        }
        Ok(())
    }

    /// 切换当前 CoverFlow 卡片并按需自动起播第一首。
    fn seldeck(&mut self, index: usize, auto_play: bool) -> Result<(), AppError> {
        if index >= self.decks.len() { return Err(CatalogError::NotFound.into()); }
        self.active_deck = Some(index);
        if auto_play {
            self.lddeck(index)?;
        }
        self.synqu(index)?;
        if auto_play {
            if let Some(first) = self.decks[index].tracks.first().cloned() {
                self.queue.stcrr(0);
                self.decks[index].cursor = Some(0);
                self.pltrk(first.id)?;
            } else {
                self.playback.stp();
                self.snapshot.current = None;
            }
        }
        Ok(())
    }

    /// 将当前 CoverFlow 卡片曲目同步到活动播放队列。
    fn synqu(&mut self, index: usize) -> Result<(), AppError> {
        let Some(deck) = self.decks.get(index) else { return Ok(()); };
        self.queue.items = deck.tracks.iter().map(|t| t.id.clone()).collect();
        self.queue.cursor = if self.queue.items.is_empty() {
            None
        } else {
            Some(deck.cursor.unwrap_or(0).min(self.queue.items.len() - 1))
        };
        self.queue.version = self.queue.version.wrapping_add(1);
        self.svqueue()
    }

    /// 将自定义封面插入到日推右侧与推荐歌单左侧并持久化。
    fn insflow(&mut self, deck: CoverDeck) -> Result<(), AppError> {
        let pos = self.decks.iter().position(|d| d.kind == DeckKind::DailyRecommend).map_or(0, |i| i + 1);
        // 插入后按id重定位。
        let active_id = self.active_deck.and_then(|i| self.decks.get(i)).map(|d| d.id.clone());
        if let Some(existing) = self.decks.iter().position(|d| d.id == deck.id) {
            self.decks.remove(existing);
        }
        let insert_at = pos.min(self.decks.len());
        self.decks.insert(insert_at, deck);
        if let Some(active_id) = active_id {
            self.active_deck = self.decks.iter().position(|d| d.id == active_id);
        }
        self.svdckst()
    }

    /// 剔除用户添加至 CoverFlow 的封面卡片。
    fn rmvdeck(&mut self, index: usize) -> Result<(), AppError> {
        let Some(deck) = self.decks.get(index) else { return Err(CatalogError::NotFound.into()); };
        if !deck.removable {
            return Err(CatalogError::InvalidData("默认歌单不可从CoverFlow移除".into()).into());
        }
        self.decks.remove(index);
        self.svdckst()?;
        if let Some(cur) = self.active_deck {
            if cur == index {
                self.playback.stp();
                self.snapshot.current = None;
                self.lyrics.clear();
                if self.decks.is_empty() {
                    self.active_deck = None;
                    self.queue.clr();
                    self.svqueue()?;
                } else {
                    let next = index.min(self.decks.len() - 1);
                    self.active_deck = Some(next);
                    self.synqu(next)?;
                }
            } else if cur > index {
                self.active_deck = Some(cur - 1);
            }
        }
        Ok(())
    }

    /// 保存全部用户添加的 CoverFlow 卡片。
    fn svdckst(&self) -> Result<(), AppError> {
        let custom: Vec<CoverDeck> = self.decks.iter().filter(|d| d.removable).cloned().collect();
        self.storage.svdecks(&custom)
    }

    /// 准备「添加至」二级弹窗内容（单曲或整张专辑）。
    fn opnadd(&mut self, search_index: usize, whole_album: bool) -> Result<(), AppError> {
        let track = self.snapshot.search_results.get(search_index).cloned().ok_or(CatalogError::NotFound)?;
        if self.user_playlists.is_empty() && matches!(self.account.stt(), AccountState::Authorized { .. }) {
            if let Ok(pls) = self.library.usrpls() { self.user_playlists = pls; }
        }
        self.add_to_prompt = if whole_album {
            let aid = track.album_id.as_deref().filter(|s| !s.is_empty()).ok_or_else(|| CatalogError::InvalidData("该单曲缺少专辑信息".into()))?;
            let (alb_name, alb_cover, tracks) = self.catalog.albdtl(aid)?;
            if tracks.is_empty() { return Err(CatalogError::NotFound.into()); }
            self.storage.svtrks(&tracks)?;
            Some(AddToPrompt {
                title: alb_name,
                cover_key: alb_cover.or(track.cover_key.clone()),
                source_id: aid.to_string(),
                whole_album: true,
                tracks,
                created_playlist_id: None,
            })
        } else {
            let mut trk = track.clone();
            if trk.album_name.is_none() { trk.album_name = Some(trk.title.clone()); }
            Some(AddToPrompt {
                title: trk.title.clone(),
                cover_key: trk.cover_key.clone(),
                source_id: trk.id.raw_id.clone(),
                whole_album: false,
                tracks: vec![trk],
                created_playlist_id: None,
            })
        };
        Ok(())
    }

    /// 新建用户云端歌单并同步到收藏列表与当前弹窗。
    fn crtpl(&mut self, name: &str) -> Result<(), AppError> {
        let created = self.library.mkpl(name)?;
        let mut pls = self.library.usrpls().unwrap_or_default();
        if !pls.iter().any(|p| p.id == created.id) {
            pls.push(created.clone());
        }
        self.user_playlists = pls;
        if let Some(ref mut prompt) = self.add_to_prompt {
            prompt.created_playlist_id = Some(created.id);
        }
        Ok(())
    }

    /// 提交「添加至」多选结果。
    fn aplyadd(&mut self, to_coverflow: bool, to_like: bool, playlist_ids: &[String]) -> Result<(), AppError> {
        let prompt = self.add_to_prompt.take().ok_or(CatalogError::NotFound)?;
        self.storage.svtrks(&prompt.tracks)?;
        if to_coverflow {
            let deck = if prompt.whole_album {
                CoverDeck {
                    id: format!("album:{}", prompt.source_id),
                    kind: DeckKind::Album,
                    title: prompt.title.clone(),
                    cover_key: prompt.cover_key.clone(),
                    source_id: prompt.source_id.clone(),
                    removable: true,
                    tracks_loaded: true,
                    tracks: prompt.tracks.clone(),
                    cursor: Some(0),
                }
            } else {
                CoverDeck {
                    id: format!("single:{}", prompt.source_id),
                    kind: DeckKind::Single,
                    title: prompt.title.clone(),
                    cover_key: prompt.cover_key.clone(),
                    source_id: prompt.source_id.clone(),
                    removable: true,
                    tracks_loaded: true,
                    tracks: prompt.tracks.clone(),
                    cursor: Some(0),
                }
            };
            self.insflow(deck)?;
        }
        if to_like {
            if !self.snapshot.favorites_local {
                if let Some(like_lid) = self.user_playlists.iter().find(|p| p.is_like).and_then(|p| p.list_id) {
                    self.library.addtrks(like_lid, &prompt.tracks)?;
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                    loop {
                        self.snapshot.favorites = self.library.fvrts()?;
                        if prompt.tracks.iter().all(|t| self.snapshot.favorites.iter().any(|f| f.id == t.id)) { break; }
                        if std::time::Instant::now() >= deadline {
                            for t in &prompt.tracks {
                                if !self.snapshot.favorites.iter().any(|f| f.id == t.id) {
                                    self.snapshot.favorites.push(t.clone());
                                }
                            }
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(300));
                    }
                    self.storage.svtrks(&self.snapshot.favorites)?;
                } else {
                    for trk in &prompt.tracks { self.tglfvrt_inner(trk)?; }
                }
            } else {
                for trk in &prompt.tracks {
                    if !self.snapshot.favorites.iter().any(|f| f.id == trk.id) {
                        self.storage.stfvrt(&trk.id, true)?;
                    }
                }
                self.snapshot.favorites = self.storage.ldfvrt()?;
            }
            self.synldck();
        }
        for pid in playlist_ids {
            if let Ok(lid) = pid.parse::<i64>() {
                self.library.addtrks(lid, &prompt.tracks)?;
                if let Some(meta) = self.user_playlists.iter_mut().find(|p| p.list_id == Some(lid)) {
                    meta.song_count = meta.song_count.saturating_add(prompt.tracks.len() as u32);
                }
                if let Some(d) = self.decks.iter_mut().find(|d| d.kind == DeckKind::UserPlaylist && d.source_id == *pid) {
                    d.tracks.extend(prompt.tracks.iter().cloned());
                }
            }
        }
        if let Ok(pls) = self.library.usrpls() {
            for p in &pls {
                if !self.user_playlists.iter().any(|e| e.id == p.id) {
                    self.user_playlists.push(p.clone());
                }
            }
            for e in &mut self.user_playlists {
                if let Some(fresh) = pls.iter().find(|p| p.id == e.id) {
                    e.song_count = e.song_count.max(fresh.song_count);
                    e.cover_key = fresh.cover_key.clone().or_else(|| e.cover_key.clone());
                }
            }
        }
        Ok(())
    }

    /// 打开收藏页二级歌单曲目列表。
    fn opnpl(&mut self, id: &str) -> Result<(), AppError> {
        let meta = self.user_playlists.iter().find(|p| p.id == id).cloned().ok_or(CatalogError::NotFound)?;
        let tracks = if meta.is_like {
            self.rfrshfvrt()?;
            self.snapshot.favorites.clone()
        } else if let Some(lid) = meta.list_id {
            self.library.pltrks(lid)?
        } else if let Some(ref gid) = meta.global_id {
            self.catalog.pubpltrks(gid)?
        } else {
            Vec::new()
        };
        self.storage.svtrks(&tracks)?;
        self.opened_playlist = Some(OpenedPlaylist { meta, tracks });
        Ok(())
    }

    /// 从收藏页将歌单加入 CoverFlow 或直接播放。
    fn plypl(&mut self, id: &str, play_now: bool) -> Result<(), AppError> {
        let meta = self.user_playlists.iter().find(|p| p.id == id).cloned().ok_or(CatalogError::NotFound)?;
        if meta.is_like {
            if play_now {
                if let Some(idx) = self.decks.iter().position(|d| d.kind == DeckKind::Like) {
                    self.seldeck(idx, true)?;
                    self.snapshot.deck_flow_seq = self.snapshot.deck_flow_seq.wrapping_add(1);
                }
            }
            return Ok(());
        }
        let tracks = if self.opened_playlist.as_ref().is_some_and(|o| o.meta.id == id) {
            self.opened_playlist.as_ref().unwrap().tracks.clone()
        } else if let Some(lid) = meta.list_id {
            self.library.pltrks(lid)?
        } else if let Some(ref gid) = meta.global_id {
            self.catalog.pubpltrks(gid)?
        } else {
            Vec::new()
        };
        self.storage.svtrks(&tracks)?;
        let deck = CoverDeck {
            id: Self::pldkid(&meta.id),
            kind: DeckKind::UserPlaylist,
            title: meta.name.clone(),
            cover_key: meta.cover_key.clone().or_else(|| tracks.first().and_then(|t| t.cover_key.clone())),
            source_id: meta.list_id.map(|v| v.to_string()).unwrap_or(meta.id),
            removable: true,
            tracks_loaded: true,
            tracks,
            cursor: Some(0),
        };
        let deck_id = deck.id.clone();
        self.insflow(deck)?;
        if play_now {
            if let Some(idx) = self.decks.iter().position(|d| d.id == deck_id) {
                self.seldeck(idx, true)?;
                self.snapshot.deck_flow_seq = self.snapshot.deck_flow_seq.wrapping_add(1);
            }
        }
        Ok(())
    }

    /// 将搜索结果曲目加入指定歌单并立即播放；prepend 置于歌单首（插播），否则追加尾部（续播）。
    fn addplplay(&mut self, search_index: usize, playlist_id: &str, prepend: bool) -> Result<(), AppError> {
        let track = self.snapshot.search_results.get(search_index).cloned().ok_or(CatalogError::NotFound)?;
        let meta = self.user_playlists.iter().find(|p| p.id == playlist_id).cloned().ok_or(CatalogError::NotFound)?;
        if let Some(lid) = meta.list_id { self.library.addtrks(lid, std::slice::from_ref(&track))?; }
        self.storage.svtrks(std::slice::from_ref(&track))?;
        let deck_id = if meta.is_like { "like".to_string() } else { Self::pldkid(&meta.id) };
        if let Some(idx) = self.decks.iter().position(|d| d.id == deck_id) {
            if !self.decks[idx].tracks_loaded { self.lddeck(idx)?; }
            let play_idx = if prepend {
                self.decks[idx].tracks.insert(0, track.clone()); 0
            } else {
                self.decks[idx].tracks.push(track.clone()); self.decks[idx].tracks.len().saturating_sub(1)
            };
            self.decks[idx].cursor = Some(play_idx);
            self.decks[idx].tracks_loaded = true;
            self.svdckst()?;
            self.seldeck(idx, false)?;
            self.pltrk(track.id.clone())?;
        } else {
            let mut tracks = if let Some(lid) = meta.list_id { self.library.pltrks(lid)? } else { Vec::new() };
            if let Some(pos) = tracks.iter().position(|t| t.id == track.id) { tracks.remove(pos); }
            let play_idx = if prepend { tracks.insert(0, track.clone()); 0 } else { tracks.push(track.clone()); tracks.len().saturating_sub(1) };
            let deck = CoverDeck {
                id: deck_id.clone(),
                kind: if meta.is_like { DeckKind::Like } else { DeckKind::UserPlaylist },
                title: meta.name.clone(),
                cover_key: meta.cover_key.clone().or_else(|| track.cover_key.clone()),
                source_id: meta.list_id.map(|v| v.to_string()).unwrap_or_else(|| meta.id.clone()),
                removable: !meta.is_like,
                tracks_loaded: true,
                tracks,
                cursor: Some(play_idx),
            };
            self.insflow(deck)?;
            let idx = self.decks.iter().position(|d| d.id == deck_id).ok_or(CatalogError::NotFound)?;
            self.seldeck(idx, false)?;
            self.pltrk(track.id.clone())?;
        }
        self.snapshot.deck_flow_seq = self.snapshot.deck_flow_seq.wrapping_add(1);
        if self.opened_playlist.as_ref().is_some_and(|o| o.meta.id == meta.id) { self.opnpl(&meta.id)?; }
        Ok(())
    }

    /// 打开搜索曲目的专辑视图页。
    fn viewalbm(&mut self, search_index: usize) -> Result<(), AppError> {
        let track = self.snapshot.search_results.get(search_index).cloned().ok_or(CatalogError::NotFound)?;
        let aid = track.album_id.clone().filter(|s| !s.is_empty())
            .ok_or_else(|| CatalogError::InvalidData("该单曲缺少专辑信息".into()))?;
        let (name, _, tracks) = self.catalog.albdtl(&aid)?;
        self.storage.svtrks(&tracks)?;
        self.snapshot.album_view = Some(AlbumView { title: name, source_id: aid.to_string(), tracks });
        Ok(())
    }

    /// 下载单曲歌词文件至下载目录。
    fn dnlyrc(&mut self, id: &TrackId) -> Result<(), AppError> {
        let track = self.trksmry(id)?;
        let text = self.catalog.lyrtxt(id)?;
        let base = if self.settings.downloads.directory.as_os_str().is_empty() {
            crate::services::downloads::dwnlddir()?
        } else {
            self.settings.downloads.directory.clone()
        };
        let name = crate::services::downloads::flname(&format!("{} - {}", track.artist_text, track.title));
        let path = base.join(format!("{name}.lrc"));
        std::fs::write(&path, text).map_err(|error| StorageError::Io(error.to_string()))?;
        self.snapshot.notice = Some(format!("已下载歌词：{}", path.display()));
        Ok(())
    }

    /// 从打开的账号歌单移除指定曲目。
    fn rmpltrk(&mut self, index: usize) -> Result<(), AppError> {
        let (meta_id, list_id, track) = {
            let opened = self.opened_playlist.as_ref().ok_or(CatalogError::NotFound)?;
            let track = opened.tracks.get(index).ok_or(CatalogError::NotFound)?.clone();
            (opened.meta.id.clone(), opened.meta.list_id, track)
        };
        let lid = list_id.ok_or(CatalogError::NotFound)?;
        self.library.deltrk(lid, &track.id)?;
        self.opnpl(&meta_id)?;
        if let Some(deck) = self.decks.iter_mut().find(|d| d.kind == DeckKind::UserPlaylist && d.source_id == meta_id) {
            if let Some(opened) = &self.opened_playlist { deck.tracks = opened.tracks.clone(); deck.tracks_loaded = true; }
        }
        Ok(())
    }

    /// 载入打开的歌单为队列并从指定曲目播放。
    fn plypltrk(&mut self, index: usize) -> Result<(), AppError> {
        let meta_id = self.opened_playlist.as_ref().ok_or(CatalogError::NotFound)?.meta.id.clone();
        let track = self.opened_playlist.as_ref().ok_or(CatalogError::NotFound)?
            .tracks.get(index).cloned().ok_or(CatalogError::NotFound)?;
        let deck_id = if self.opened_playlist.as_ref().is_some_and(|o| o.meta.is_like) { "like".to_string() } else { Self::pldkid(&meta_id) };
        let idx = self.decks.iter().position(|d| d.id == deck_id).ok_or(CatalogError::NotFound)?;
        if !self.decks[idx].tracks_loaded { self.lddeck(idx)?; }
        self.decks[idx].cursor = Some(index);
        self.active_deck = Some(idx);
        self.synqu(idx)?;
        self.pltrk(track.id)?;
        self.snapshot.deck_flow_seq = self.snapshot.deck_flow_seq.wrapping_add(1);
        Ok(())
    }

    /// 载入专辑视图对应的专辑为队列并从指定曲目播放。
    fn albmplay(&mut self, index: usize) -> Result<(), AppError> {
        let (aid, track) = {
            let view = self.snapshot.album_view.as_ref().ok_or(CatalogError::NotFound)?;
            let track = view.tracks.get(index).cloned().ok_or(CatalogError::NotFound)?;
            (view.source_id.clone(), track)
        };
        let deck_id = format!("album:{}", aid);
        let idx = if let Some(i) = self.decks.iter().position(|d| d.id == deck_id) {
            if !self.decks[i].tracks_loaded { self.lddeck(i)?; }
            i
        } else {
            let (name, cover, tracks) = self.catalog.albdtl(&aid)?;
            self.storage.svtrks(&tracks)?;
            let deck = CoverDeck {
                id: deck_id.clone(),
                kind: DeckKind::Album,
                title: name,
                cover_key: cover,
                source_id: aid,
                removable: true,
                tracks_loaded: true,
                tracks,
                cursor: None,
            };
            self.insflow(deck)?;
            self.decks.iter().position(|d| d.id == deck_id).ok_or(CatalogError::NotFound)?
        };
        if self.decks[idx].tracks.is_empty() { return Err(CatalogError::NotFound.into()); }
        let play_idx = index.min(self.decks[idx].tracks.len() - 1);
        self.decks[idx].cursor = Some(play_idx);
        self.seldeck(idx, false)?;
        self.pltrk(track.id)?;
        self.snapshot.deck_flow_seq = self.snapshot.deck_flow_seq.wrapping_add(1);
        Ok(())
    }

    /// 删除自建歌单并清理歌单列表、CoverFlow卡片与打开状态。
    fn dellst(&mut self, id: &str) -> Result<(), AppError> {
        if id == "like" { return Err(CatalogError::InvalidData("收藏歌单不可删除".into()).into()); }
        let meta = self.user_playlists.iter().find(|p| p.id == id).cloned().ok_or(CatalogError::NotFound)?;
        let lid = meta.list_id.ok_or(CatalogError::NotFound)?;
        self.library.dellst(lid)?;
        self.user_playlists.retain(|p| p.id != id);
        if let Some(idx) = self.decks.iter().position(|d| d.id == Self::pldkid(id)) {
            self.rmvdeck(idx)?;
        }
        if self.opened_playlist.as_ref().is_some_and(|o| o.meta.id == id) {
            self.opened_playlist = None;
        }
        Ok(())
    }

    /// 按专辑标识打开专辑视图页。
    fn viewalbm_id(&mut self, aid: &str) -> Result<(), AppError> {
        let (name, _, tracks) = self.catalog.albdtl(aid)?;
        self.storage.svtrks(&tracks)?;
        self.snapshot.album_view = Some(AlbumView { title: name, source_id: aid.to_string(), tracks });
        Ok(())
    }

    /// 底部爱心按钮点击：未喜欢直接收藏，已喜欢弹出确认框。
    fn hrtclk(&mut self) -> Result<(), AppError> {
        let Some(track) = self.snapshot.current.clone() else { return Ok(()); };
        if self.snapshot.is_favorite {
            let list_name = self.user_playlists.iter().find(|p| p.is_like).map(|p| p.name.as_str()).unwrap_or("Like");
            self.confirm_unlike = Some((format!("确认从歌单“{list_name}”中删除这首歌吗？"), track));
        } else {
            self.tglfvrt_inner(&track)?;
        }
        Ok(())
    }

    /// 确认是否从 Like 歌单删除待定歌曲。
    fn unlkcf(&mut self, confirm: bool) -> Result<(), AppError> {
        let pending = self.confirm_unlike.take();
        if confirm {
            if let Some((_, track)) = pending {
                if self.snapshot.favorites.iter().any(|favorite| favorite.id == track.id) {
                    self.tglfvrt_inner(&track)?;
                }
            }
        }
        Ok(())
    }

    /// 探测下载品质选项并打开「下载品质」二级弹窗。
    fn inspdl(&mut self, scope: TargetScope, search_index: Option<usize>, whole_album: bool) -> Result<(), AppError> {
        let (collection_name, tracks) = if let Some(sidx) = search_index {
            let track = self.snapshot.search_results.get(sidx).cloned().ok_or(CatalogError::NotFound)?;
            if whole_album {
                let aid = track.album_id.as_deref().filter(|s| !s.is_empty()).ok_or_else(|| CatalogError::InvalidData("该单曲缺少专辑信息".into()))?;
                let (name, _, trks) = self.catalog.albdtl(aid)?;
                (name, trks)
            } else {
                let name = track.collname();
                (name, vec![track])
            }
        } else {
            match scope {
                TargetScope::Single => {
                    let track = self.snapshot.current.clone().ok_or(CatalogError::NotFound)?;
                    let name = track.album_name.clone()
                        .or_else(|| self.active_deck.and_then(|i| self.decks.get(i)).map(|d| d.title.clone()))
                        .unwrap_or_else(|| track.colttl());
                    (name, vec![track])
                }
                TargetScope::Collection => {
                    if let Some(idx) = self.active_deck {
                        self.lddeck(idx)?;
                        let deck = &self.decks[idx];
                        if deck.tracks.is_empty() { return Err(CatalogError::NotFound.into()); }
                        (deck.title.clone(), deck.tracks.clone())
                    } else if let Some(track) = self.snapshot.current.clone() {
                        if let Some(ref aid) = track.album_id {
                            let (name, _, trks) = self.catalog.albdtl(aid)?;
                            (name, trks)
                        } else {
                            return Err(CatalogError::NotFound.into());
                        }
                    } else {
                        return Err(CatalogError::NotFound.into());
                    }
                }
            }
        };
        if tracks.is_empty() { return Err(CatalogError::NotFound.into()); }
        let available = self.catalog.prvqlt(&tracks)?;
        self.download_prompt = Some(DownloadPrompt { collection_name, tracks, available });
        Ok(())
    }

    /// 按选定品质发起单曲或专辑/歌单分目录下载。
    fn cfmdl(&mut self, quality: crate::domain::track::AudioQuality) -> Result<(), AppError> {
        let prompt = self.download_prompt.take().ok_or(CatalogError::NotFound)?;
        if self.download.as_ref().is_some_and(|task| !task.done()) {
            return Err(CatalogError::InvalidData("已有下载正在进行".into()).into());
        }
        if !prompt.available.contains(&quality) {
            return Err(CatalogError::InvalidData("所选音质不可用".into()).into());
        }
        self.storage.svtrks(&prompt.tracks)?;
        self.download = Some(crate::services::downloads::DownloadTask::btch(
            prompt.tracks,
            prompt.collection_name,
            quality,
            self.settings.downloads.directory.clone(),
            self.stream.clone(),
            Some(self.storage.clone()),
        )?);
        Ok(())
    }

    /// 切换云端或本地收藏状态。
    fn tglfvrt_inner(&mut self, track: &TrackSummary) -> Result<(), AppError> {
        let id = track.id.clone();
        if !self.snapshot.favorites_local {
            match self.library.tglfvrt(track) {
                Ok(expected) => {
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                    loop {
                        self.snapshot.favorites = self.library.fvrts()?;
                        if self.snapshot.favorites.iter().any(|track| track.id == id) == expected { break; }
                        if std::time::Instant::now() >= deadline { return Err(CatalogError::InvalidData("云端收藏结果尚未确认".into()).into()); }
                        std::thread::sleep(std::time::Duration::from_millis(300));
                    }
                    self.storage.svtrks(&self.snapshot.favorites)?;
                    self.synldck();
                    return Ok(());
                }
                Err(e) => {
                    self.snapshot.error = Some(format!("云端收藏不可用，已使用本地收藏：{e}"));
                    self.snapshot.favorites_local = true;
                    self.snapshot.favorites = self.storage.ldfvrt()?;
                }
            }
        }
        let enabled = !self.snapshot.favorites.iter().any(|item| item.id == id);
        self.storage.svtrks(std::slice::from_ref(track))?;
        self.storage.stfvrt(&id, enabled)?;
        self.snapshot.favorites = self.storage.ldfvrt()?;
        self.synldck();
        Ok(())
    }

    /// 将当前收藏同步到 Like 卡片。
    fn synldck(&mut self) {
        if let Some(deck) = self.decks.iter_mut().find(|d| d.kind == DeckKind::Like) {
            deck.tracks = self.snapshot.favorites.clone();
            deck.tracks_loaded = true;
        }
    }

    /// 刷新云端收藏并保留本地回退。
    fn rfrshfvrt(&mut self) -> Result<(), AppError> {
        if matches!(self.account.stt(), AccountState::Authorized { .. }) {
            match self.library.fvrts() {
                Ok(tracks) => {
                    self.storage.svtrks(&tracks)?;
                    self.snapshot.favorites = tracks;
                    self.snapshot.favorites_local = false;
                }
                Err(error) => {
                    self.snapshot.favorites = self.storage.ldfvrt()?;
                    self.snapshot.favorites_local = true;
                    self.snapshot.error = Some(format!("账号收藏不可用，已显示本地收藏：{error}"));
                }
            }
        } else {
            self.snapshot.favorites = self.storage.ldfvrt()?;
            self.snapshot.favorites_local = true;
        }
        Ok(())
    }

    /// 复用已保存的曲目概要。
    fn trksmry(&self, id: &TrackId) -> Result<TrackSummary, AppError> {
        if id.raw_id.trim().is_empty() { return Err(CatalogError::NotFound.into()); }
        if let Some(track) = self.storage.ldtrk(id)? { return Ok(track); }
        let track = self.catalog.trkdtl(id)?.tosmry();
        self.storage.svtrks(std::slice::from_ref(&track))?;
        Ok(track)
    }

    /// 解析成功后切换实际播放与队列游标（无损本地优先，断网时回退任意本地已下载音频）。
    fn pltrk(&mut self, id: TrackId) -> Result<(), AppError> {
        let track = self.trksmry(&id)?;
        let local_any = self.storage.lddwnld(&id)?.filter(|dl| std::path::Path::new(&dl.file_path).is_file());
        let lclsrc = |dl: crate::domain::track::DownloadedFile| -> ResolvedSource {
            let ext = std::path::Path::new(&dl.file_path)
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_ascii_lowercase());
            ResolvedSource {
                url_or_local: dl.file_path,
                codec_hint: ext,
                            }
        };
        let source = if local_any.as_ref().is_some_and(|dl| dl.is_lossless) {
            lclsrc(local_any.unwrap())
        } else {
            match self.stream.rslvply(&track, self.settings.audio.quality) {
                Ok(src) => src,
                Err(err) => match local_any {
                    Some(dl) => lclsrc(dl),
                    None => return Err(err),
                },
            }
        };
        self.playback.ply(source)?;
        let index = self.queue.cursor.filter(|index| self.queue.items.get(*index) == Some(&id))
            .or_else(|| self.queue.items.iter().position(|item| item == &id)).unwrap_or_else(|| {
            self.queue.psh(id);
            self.queue.items.len() - 1
        });
        self.queue.stcrr(index);
        if let Some(d) = self.active_deck.and_then(|i| self.decks.get_mut(i)) {
            d.cursor = Some(index);
        }
        let changed = self.snapshot.current.as_ref().is_none_or(|current| current.id != track.id);
        self.snapshot.current = Some(track);
        self.end_handled = false;
        if changed { self.lyrics.clear(); }
        self.svqueue()
    }

    /// 前后切歌失败时恢复队列游标；顺序模式末曲结束后自动 flow 至下一 Cover。
    fn mvtrk(&mut self, forward: bool) -> Result<(), AppError> {
        let previous = self.queue.cursor;
        let next = if forward { self.queue.nxt() } else { self.queue.prv() }.cloned();
        if let Some(id) = next {
            if let Err(error) = self.pltrk(id) {
                self.queue.cursor = previous;
                return Err(error);
            }
        } else if forward && self.queue.repeat_mode == RepeatMode::Off {
            if let Some(cur) = self.active_deck {
                if cur + 1 < self.decks.len() {
                    self.seldeck(cur + 1, true)?;
                    self.snapshot.deck_flow_seq = self.snapshot.deck_flow_seq.wrapping_add(1);
                    return Ok(());
                }
            }
            self.playback.stp();
        } else {
            self.playback.stp();
        }
        Ok(())
    }

    /// 保存队列变更。
    fn svqueue(&self) -> Result<(), AppError> {
        self.storage.svqu(&self.queue.items, self.queue.cursor)
    }

    /// 返回用户歌单卡片标识。
fn pldkid(id: &str) -> String { format!("user_pl:{id}") }

/// 应用仍然对应当前歌曲的异步歌词。
    pub fn stlyrc(&mut self, id: &TrackId, lines: Vec<LyricLine>) {
        if self.settings.lyrics.desktop_enabled && self.snapshot.current.as_ref().is_some_and(|track| &track.id == id) {
            self.lyrics = lines;
            self.updssht();
        }
    }

    /// 更新真实进度并处理一次播放结束。
    pub fn tick(&mut self) {
        if let Some(task) = &self.download { self.snapshot.download = Some(task.state.lock().unwrap().clone()); }
        let state = self.playback.prgrss().state;
        if state == PlaybackState::Ended && !self.end_handled {
            self.end_handled = true;
            if let Err(error) = self.mvtrk(true) { self.snapshot.error = Some(error.to_string()); }
        } else if state != PlaybackState::Ended {
            self.end_handled = false;
        }
        self.updssht();
    }

    /// 刷新快照并按队列版本更新元数据。
    pub fn updssht(&mut self) {
        let progress = self.playback.prgrss();
        self.snapshot.playback = progress.state;
        if progress.state == PlaybackState::Failed && self.snapshot.error.is_none() {
            self.snapshot.error = Some("音频读取或输出失败，请重试或选择其他音质".into());
        }
        self.snapshot.position_ms = progress.position_ms;
        self.snapshot.duration_ms = progress.duration_ms;
        self.snapshot.volume = progress.volume;
        self.snapshot.account = self.account.stt();
        if matches!(self.snapshot.account, AccountState::Authorized { .. }) { self.snapshot.login_qr = None; }
        self.snapshot.checkin = self.account.chkstt();
        self.snapshot.repeat = self.queue.repeat_mode;
        self.snapshot.queue_cursor = self.queue.cursor;
        self.snapshot.shuffle = self.queue.shuffle;
        self.snapshot.theme = self.settings.appearance.theme;
        self.snapshot.settings = self.settings.clone();
        self.snapshot.is_favorite = self.snapshot.current.as_ref().is_some_and(|track|
            self.snapshot.favorites.iter().any(|favorite| favorite.id == track.id));
        self.snapshot.decks = self.decks.iter().map(|d| DeckSummary {
            id: d.id.clone(),
            kind: d.kind,
            title: d.title.clone(),
            cover_key: d.cover_key.clone(),
            removable: d.removable,
            tracks_loaded: d.tracks_loaded,
            track_count: d.tracks.len(),
        }).collect();
        self.snapshot.active_deck = self.active_deck;
        self.snapshot.user_playlists = self.user_playlists.clone();
        self.snapshot.opened_playlist = self.opened_playlist.clone();
        self.snapshot.add_to_prompt = self.add_to_prompt.clone();
        self.snapshot.confirm_unlike = self.confirm_unlike.as_ref().map(|(text, _)| text.clone());
        self.snapshot.download_prompt = self.download_prompt.clone();
        let line = self.lyrics.partition_point(|line| line.begin_ms <= progress.position_ms);
        self.snapshot.lyric_text = if self.settings.lyrics.desktop_enabled && line > 0 {
            self.lyrics[line - 1].text.clone()
        } else { String::new() };
        if self.queue_version != self.queue.version {
            let mut tracks = Vec::with_capacity(self.queue.items.len());
            for id in &self.queue.items {
                let known = match self.storage.ldtrk(id) {
                    Ok(track) => track,
                    Err(error) => { self.snapshot.error = Some(error.to_string()); None }
                };
                tracks.push(known.unwrap_or_else(|| TrackSummary::nw(id.clone(), id.raw_id.clone(), "")));
            }
            self.snapshot.queue = tracks;
            self.queue_version = self.queue.version;
        }
    }

    /// 获取当前快照。
    pub fn ssht(&self) -> &UiSnapshot { &self.snapshot }
}
