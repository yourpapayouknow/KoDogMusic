// 界面绑定核心命令与快照。
use crate::{coverflow::FlwWrkr, deps::*, view_models::{AddPlaylistItem, AppWindow, LyricWindow, MediaArt, PlaylistItem, QualityItem, Theme, TrackItem}, worker::Worker};

/// 界面回调、快照同步与封面窗口控制器。
pub struct Ctrl {
    ui: Weak<AppWindow>,
    worker: Worker,
    timer: Rc<Timer>,
    state: UiSnapshot,
    tray: Option<crate::native::Tray>,
    lyrics: Option<LyricWindow>,
    lyrics_hooked: bool,
    covers: FlwWrkr,
    frame: Option<Arc<Frame>>,
    like_art: Arc<RgbaImage>,
    daily_art: Arc<RgbaImage>,
    fallback_art: Arc<RgbaImage>,
    flwslots: Vec<(usize, Option<Arc<RgbaImage>>, Option<[u8; 3]>)>,
    flwtotal: usize,
    flwtarget: usize,
    flwmiss: bool,
    flwbg: [u8; 3],
    fast_timer: bool,
    expected_cursor: Option<(usize, Instant)>,
    wheel_commit: Option<(usize, Instant)>,
    last_login_flow: u64,
    last_deck_flow: u64,
    add_playlist_selected: Vec<String>,
    add_prompt_key: Option<(String, bool)>,
    pick_pending: Option<(usize, bool)>,
    pick_selected: Option<String>,
    dl_selected_q: i32,
    dl_prompt_key: Option<String>,
    qr: Option<Arc<Frame>>,
    qr_img: Option<Arc<RgbaImage>>,
    note: String,
    noted: Instant,
    slider: Instant,
    synced: Instant,
    hooked: bool,
    scale: f32,
    dark: Option<bool>,
}

/// 绑定界面回调与同一个核心线程。
pub fn bind(ui: &AppWindow, worker: Worker, timer: Rc<Timer>) -> Rc<RefCell<Ctrl>> {
    let tray = crate::native::Tray::nw();
    let error = tray.as_ref().err().cloned();
    let scale = if ui.window().scale_factor().is_finite() && ui.window().scale_factor() > 0. { ui.window().scale_factor() } else { 1. };
    let dark = ui.global::<Theme>().get_dark();
    let flwbg = if dark { [32, 39, 44] } else { [241, 235, 229] };
    let covers = FlwWrkr::nw((360. * scale).ceil() as u32, (274. * scale).ceil() as u32, flwbg);
    let initial = covers.pllfrm();
    ui.set_coverflow_image(frmimg(&initial));
    let ctrl = Rc::new(RefCell::new(Ctrl {
        ui: ui.as_weak(), worker, timer, state: UiSnapshot::default(), tray: tray.ok(), lyrics: None, lyrics_hooked: false,
        covers, frame: initial,
        like_art: crate::coverflow::mkart(DeckKind::Like),
        daily_art: crate::coverflow::mkart(DeckKind::DailyRecommend),
        fallback_art: crate::coverflow::mkart(DeckKind::RecommendedPlaylist),
        flwslots: Vec::new(), flwtotal: 0, flwtarget: 0, flwmiss: false, flwbg,
        fast_timer: false, expected_cursor: None, wheel_commit: None,
        last_login_flow: 0, last_deck_flow: 0,
        add_playlist_selected: Vec::new(), add_prompt_key: None,
        pick_pending: None, pick_selected: None,
        dl_selected_q: 1, dl_prompt_key: None,
        qr: None, qr_img: None,
        note: error.unwrap_or_default(), noted: Instant::now(), slider: Instant::now(), synced: Instant::now() - Duration::from_secs(1),
        hooked: false, scale, dark: None,
    }));
    let cache = ctrl.borrow().worker.art.clone();
    ui.global::<MediaArt>().on_cover(move |url, size, _| {
        cache.get(url.as_str(), size.clamp(1,128) as u32).and_then(|art| art.frame).map(|frame| frameimg(&frame)).unwrap_or_default()
    });
    let c = ctrl.clone(); ui.on_command(move |a| c.borrow_mut().cmd(a));
    let c = ctrl.clone(); ui.on_nav(move |a| { let mut c = c.borrow_mut(); c.snd(AppCommand::Nav(match a { 1 => AppRoute::Queue, 2 => AppRoute::Search, 3 => AppRoute::Favorites, 4 => AppRoute::Settings, _ => AppRoute::Player })); c.wakflw(); });
    let c = ctrl.clone(); ui.on_search(move |query| { let mut c = c.borrow_mut(); if !query.trim().is_empty() { c.snd(AppCommand::Search(query.to_string())); } });
    let c = ctrl.clone(); ui.on_row_action(move |s,i,a| c.borrow_mut().rowact(s,i,a));
    let c = ctrl.clone(); ui.on_cover_drag(move |dx,phase,mx| c.borrow_mut().covdrg(dx,phase,mx));
    let c = ctrl.clone(); ui.on_cover_wheel(move |dy| c.borrow_mut().covwhl(dy));
    let c = ctrl.clone(); ui.on_seek(move |v,done| c.borrow_mut().slide(v,done,false));
    let c = ctrl.clone(); ui.on_set_volume(move |v,done| c.borrow_mut().slide(v,done,true));
    let c = ctrl.clone(); ui.on_setting(move |s,v| c.borrow_mut().stpref(s,v));
    let c = ctrl.clone(); ui.on_directory(move |kind,path| {
        let path = PathBuf::from(path.trim());
        if !path.is_absolute() || !matches!(kind,4|5) { return false; }
        // 目录预校验后立即报错。
        if std::fs::create_dir_all(&path).is_err() { return false; }
        c.borrow_mut().snd(if kind == 4 { AppCommand::SetCacheDir(path) } else { AppCommand::SetDownloadDir(path) });
        true
    });
    let c = ctrl.clone(); ui.on_open_add_to(move |idx, whole_album| {
        if let Ok(search_index) = usize::try_from(idx) {
            c.borrow_mut().snd(AppCommand::OpenAddTo { search_index, whole_album });
        }
    });
    let c = ctrl.clone(); ui.on_cancel_add_to(move || c.borrow_mut().snd(AppCommand::CancelAddTo));
    let c = ctrl.clone(); ui.on_create_playlist(move |name| {
        let trimmed = name.trim();
        if !trimmed.is_empty() {
            c.borrow_mut().snd(AppCommand::CreatePlaylist(trimmed.to_string()));
        }
    });
    let c = ctrl.clone(); ui.on_toggle_add_playlist(move |idx| c.borrow_mut().tgladdpl(idx));
    let c = ctrl.clone(); ui.on_confirm_add_to(move |to_flow, to_like| c.borrow_mut().cfrmadd(to_flow, to_like));
    let c = ctrl.clone(); ui.on_open_playlist(move |id| c.borrow_mut().snd(AppCommand::OpenPlaylist(id.to_string())));
    let c = ctrl.clone(); ui.on_delete_playlist(move |id| c.borrow_mut().snd(AppCommand::DeletePlaylist(id.to_string())));
    let c = ctrl.clone(); ui.on_close_playlist(move || c.borrow_mut().snd(AppCommand::ClosePlaylist));
    let c = ctrl.clone(); ui.on_playlist_action(move |act| c.borrow_mut().plact(act));
    let c = ctrl.clone(); ui.on_confirm_unlike(move |ok| c.borrow_mut().snd(AppCommand::ConfirmUnlike(ok)));
    let c = ctrl.clone(); ui.on_queue_clear(move || c.borrow_mut().snd(AppCommand::QueueClear));
    let c = ctrl.clone(); ui.on_refresh_favorites(move || c.borrow_mut().snd(AppCommand::RefreshFavorites));
    let c = ctrl.clone(); ui.on_open_add_play(move |idx, prepend| c.borrow_mut().opnplpay(idx, prepend));
    let c = ctrl.clone(); ui.on_pick_playlist(move |idx| c.borrow_mut().pckpl(idx));
    let c = ctrl.clone(); ui.on_confirm_add_play(move || c.borrow_mut().cfrmplpay());
    let c = ctrl.clone(); ui.on_cancel_add_play(move || c.borrow_mut().cnclplpay());
    let c = ctrl.clone(); ui.on_open_album(move |idx| {
        if let Ok(index) = usize::try_from(idx) { c.borrow_mut().snd(AppCommand::OpenAlbum(index)); }
    });
    let c = ctrl.clone(); ui.on_close_album(move || c.borrow_mut().snd(AppCommand::CloseAlbum));
    let c = ctrl.clone(); ui.on_inspect_download(move |is_col, sidx, whole_album| {
        let scope = if is_col { TargetScope::Collection } else { TargetScope::Single };
        c.borrow_mut().snd(AppCommand::InspectDownload {
            scope,
            search_index: usize::try_from(sidx).ok(),
            whole_album,
        });
    });
    let c = ctrl.clone(); ui.on_pick_dl_quality(move |code| c.borrow_mut().pckdlq(code));
    let c = ctrl.clone(); ui.on_confirm_download(move |code| c.borrow_mut().cfrmdl(code));
    let c = ctrl.clone(); ui.on_cancel_download_prompt(move || c.borrow_mut().snd(AppCommand::CancelDownloadPrompt));
    let c = ctrl.clone(); ui.window().on_close_requested(move || { c.borrow_mut().close(); CloseRequestResponse::KeepWindowShown });
    ctrl
}

impl Ctrl {
    /// 投递有界后台命令。
    fn snd(&mut self, command: AppCommand) {
        if !self.worker.snd(command) { self.ntfy("后台繁忙，请稍后再试".into()); }
    }

    /// 提示叠加显示六秒。
    fn ntfy(&mut self, text: String) { self.note = text; self.noted = Instant::now(); }

    /// 立即同步封面窗口并刷新一帧。
    fn wakflw(&mut self) {
        if let Some(ui) = self.ui.upgrade() {
            self.synflw(&ui);
            self.stepflw(&ui);
        }
    }

    /// 切换勾选「添加至歌单」列表项。
    fn tgladdpl(&mut self, idx: i32) {
        let Ok(i) = usize::try_from(idx) else { return; };
        let writable: Vec<&UserPlaylistMeta> = self.state.user_playlists.iter().filter(|p| p.is_own && !p.is_like).collect();
        let Some(pl) = writable.get(i) else { return; };
        if let Some(pos) = self.add_playlist_selected.iter().position(|id| id == &pl.id) {
            self.add_playlist_selected.remove(pos);
        } else {
            self.add_playlist_selected.push(pl.id.clone());
        }
        if let Some(ui) = self.ui.upgrade() {
            ui.set_add_playlists(addplmdl(&self.state.user_playlists, &self.add_playlist_selected));
        }
    }

    /// 确认提交「添加至」多选结果。
    fn cfrmadd(&mut self, to_coverflow: bool, to_like: bool) {
        let playlist_ids = std::mem::take(&mut self.add_playlist_selected);
        if let Some(ui) = self.ui.upgrade() {
            ui.set_add_modal_visible(false);
        }
        self.snd(AppCommand::ConfirmAddTo { to_coverflow, to_like, playlist_ids });
    }

    /// 打开「添加至歌单并插播/续播」单选弹窗。
    fn opnplpay(&mut self, idx: i32, prepend: bool) {
        let Ok(index) = usize::try_from(idx) else { return; };
        if self.state.user_playlists.iter().filter(|p| p.is_own && !p.is_like).count() == 0 {
            self.ntfy("请先创建歌单".into());
            return;
        }
        let title = self.state.search_results.get(index).map(|t| t.title.clone()).unwrap_or_default();
        self.pick_pending = Some((index, prepend));
        self.pick_selected = None;
        if let Some(ui) = self.ui.upgrade() {
            ui.set_pick_mode(if prepend { 1 } else { 2 });
            ui.set_pick_selected(-1);
            ui.set_add_modal_title(title.into());
            ui.set_add_playlists(addplmdl(&self.state.user_playlists, &[]));
            ui.set_add_modal_visible(true);
        }
    }

    /// 单选歌单选择项。
    fn pckpl(&mut self, idx: i32) {
        let Ok(i) = usize::try_from(idx) else { return; };
        let writable: Vec<&UserPlaylistMeta> = self.state.user_playlists.iter().filter(|p| p.is_own && !p.is_like).collect();
        let Some(pl) = writable.get(i) else { return; };
        self.pick_selected = Some(pl.id.clone());
        if let Some(ui) = self.ui.upgrade() {
            ui.set_pick_selected(idx);
        }
    }

    /// 确认「插播/续播」提交。
    fn cfrmplpay(&mut self) {
        let Some((index, prepend)) = self.pick_pending.take() else { return; };
        let Some(playlist_id) = self.pick_selected.take() else { return; };
        if let Some(ui) = self.ui.upgrade() {
            ui.set_add_modal_visible(false);
            ui.set_pick_mode(0);
            ui.set_pick_selected(-1);
        }
        self.snd(AppCommand::AddPlaylistJumpPlay { search_index: index, playlist_id, prepend });
    }

    /// 取消「插播/续播」弹窗。
    fn cnclplpay(&mut self) {
        self.pick_pending = None;
        self.pick_selected = None;
        if let Some(ui) = self.ui.upgrade() {
            ui.set_add_modal_visible(false);
            ui.set_pick_mode(0);
            ui.set_pick_selected(-1);
        }
    }

    /// 处理收藏页二级歌单顶部动作（0=播放，1=加入CoverFlow）。
    fn plact(&mut self, action: i32) {
        let Some(ref opened) = self.state.opened_playlist else { return; };
        let id = opened.meta.id.clone();
        if action == 0 {
            self.snd(AppCommand::PlayPlaylist(id));
            if let Some(ui) = self.ui.upgrade() {
                ui.set_active_route(0);
            }
            self.snd(AppCommand::Nav(AppRoute::Player));
            self.wakflw();
        } else {
            self.snd(AppCommand::AddPlaylistToFlow(id));
            self.ntfy("已添加至CoverFlow".into());
        }
    }

    /// 更新下载品质弹窗选中项。
    fn pckdlq(&mut self, code: i32) {
        self.dl_selected_q = code;
        if let Some(ui) = self.ui.upgrade() {
            if let Some(ref prompt) = self.state.download_prompt {
                ui.set_dl_qualities(qltmdl(&prompt.available, self.dl_selected_q));
            }
        }
    }

    /// 确认开始下载所选品质。
    fn cfrmdl(&mut self, code: i32) {
        let selected = self.state.download_prompt.as_ref().and_then(|p| {
            let q = codetoq(code);
            if p.available.contains(&q) { Some(q) } else { p.available.first().copied() }
        }).unwrap_or_else(|| codetoq(code));
        if let Some(ui) = self.ui.upgrade() {
            ui.set_dl_modal_visible(false);
        }
        self.snd(AppCommand::ConfirmDownload(selected));
    }

    /// 按滚轮方向移动预期索引并延迟提交切换。
    fn covwhl(&mut self, dy: f32) {
        if dy.abs() < 0.01 || self.state.decks.is_empty() { return; }
        let len = self.state.decks.len();
        let cur = self.expected_cursor.map(|(c, _)| c).or(self.state.active_deck).unwrap_or(0).min(len - 1);
        let next = if dy > 0.0 {
            cur.saturating_sub(1)
        } else {
            (cur + 1).min(len - 1)
        };
        if next != cur {
            self.expected_cursor = Some((next, Instant::now()));
            self.wheel_commit = Some((next, Instant::now() + Duration::from_millis(140)));
            self.wakflw();
        }
    }

    /// 处理 CoverFlow 水平拖拽、惯性甩动吸附与两侧封面点击切换播放列表。
    fn covdrg(&mut self, dx: f32, phase: i32, mx: f32) {
        if matches!(self.state.account, AccountState::Guest) {
            if phase == 2 && (67.0..=293.0).contains(&mx) {
                if self.state.login_qr.as_ref().is_some_and(|q| q.stage == QrStage::Expired) {
                    self.snd(AppCommand::AuthLogin);
                }
            }
            return;
        }
        if self.state.decks.is_empty() { return; }
        match phase {
            0 => {
                self.wheel_commit = None;
                self.covers.drgstrt();
                self.wakflw();
            }
            1 => {
                self.covers.drgmv(dx * self.scale);
                self.wakflw();
            }
            2 => {
                if let Some(snap) = self.covers.drgrel(6.0 * self.scale) {
                    if self.state.active_deck != Some(snap) {
                        self.expected_cursor = Some((snap, Instant::now()));
                        self.snd(AppCommand::SelectDeck { index: snap, auto_play: true });
                    }
                    self.wakflw();
                } else {
                    let cur = self.state.active_deck.unwrap_or(0);
                    if mx < 67.0 && cur > 0 {
                        let next = cur - 1;
                        self.expected_cursor = Some((next, Instant::now()));
                        self.snd(AppCommand::SelectDeck { index: next, auto_play: true });
                    } else if mx > 293.0 && cur + 1 < self.state.decks.len() {
                        let next = cur + 1;
                        self.expected_cursor = Some((next, Instant::now()));
                        self.snd(AppCommand::SelectDeck { index: next, auto_play: true });
                    }
                    self.wakflw();
                }
            }
            _ => (),
        }
    }

    /// 执行常驻控件动作。
    fn cmd(&mut self, action: i32) {
        match action {
            0 => self.snd(AppCommand::TogglePlay),
            1 => self.snd(AppCommand::Prev),
            2 => self.snd(AppCommand::Next),
            3 => self.snd(AppCommand::HeartClick),
            4 => self.snd(AppCommand::SetRepeat(match self.state.repeat { RepeatMode::Off => RepeatMode::All, RepeatMode::All => RepeatMode::One, RepeatMode::One => RepeatMode::Off })),
            7 => self.snd(if matches!(self.state.account,AccountState::Guest) { AppCommand::AuthLogin } else { AppCommand::AuthLogout }),
            8 => if self.state.checkin == CheckInState::Available { self.snd(AppCommand::CheckIn); },
            9 => if let Some(ui) = self.ui.upgrade() { ui.window().set_minimized(true); },
            10 => self.close(), 11 => self.snd(AppCommand::AuthCancel),
            12 => self.snd(AppCommand::AuthLogin),
            13 => if let Some(ui) = self.ui.upgrade() { crate::native::drgwnd(ui.window()); },
            14 => if let Some(idx) = self.state.active_deck { self.snd(AppCommand::RemoveDeck(idx)); },
            _ => (),
        }
    }

    /// 关闭到通知区或退出事件循环。
    fn close(&mut self) {
        if self.state.settings.window.close_action == CloseAction::Tray && self.tray.is_some() {
            if let Some(ui) = self.ui.upgrade() { crate::native::vsbl(ui.window(),false); }
        } else { let _ = quit_event_loop(); }
    }

    /// 以队列或当前打开歌单索引派发行动作（搜索页不提供直接播放）。
    fn rowact(&mut self, scope: i32, index: i32, action: i32) {
        let fav_tracks;
        let album_tracks;
        let list: &[TrackSummary] = match scope {
            1 => &self.state.queue,
            2 => &self.state.search_results,
            3 => {
                fav_tracks = self.state.opened_playlist.as_ref().map(|o| o.tracks.as_slice()).unwrap_or(&self.state.favorites);
                fav_tracks
            }
            4 => {
                album_tracks = self.state.album_view.as_ref().map(|a| a.tracks.as_slice()).unwrap_or(&[]);
                album_tracks
            }
            _ => return,
        };
        let Some(track) = usize::try_from(index).ok().and_then(|i| list.get(i)) else { return; };
        let id = track.id.clone();
        if scope == 2 && action == 0 { return; }
        let command = match action {
            0 if scope == 1 => AppCommand::QueuePlay(index as usize),
            0 if scope == 3 => AppCommand::PlaylistPlay { index: index as usize },
            0 if scope == 4 => AppCommand::AlbumPlay { index: index as usize },
            0 => AppCommand::PlayTrack(id),
            3 => AppCommand::ToggleFavorite(id),
            4 => AppCommand::Download(id),
            6 if scope == 1 => AppCommand::QueueRemove(index as usize),
            7 if scope == 3 => AppCommand::PlaylistRemoveTrack(index as usize),
            8 if scope == 2 => AppCommand::DownloadLyrics(id),
            9 if scope == 1 || scope == 3 || scope == 4 => match track.album_id.clone() {
                Some(aid) if !aid.is_empty() => AppCommand::OpenAlbumById(aid),
                _ => return,
            },
            _ => return,
        };
        self.snd(command);
    }

    /// 合并拖动事件并保留松手时的最终值。
    fn slide(&mut self, value: f32, done: bool, volume: bool) {
        if !value.is_finite() || (!done && self.slider.elapsed() < Duration::from_millis(60)) { return; }
        self.slider = Instant::now();
        self.snd(if volume { AppCommand::SetVolume(value.clamp(0.,1.)) } else { AppCommand::Seek((value.max(0.) as u64).min(self.state.duration_ms)) });
    }

    /// 设置项复用核心持久化入口。
    fn stpref(&mut self, setting: i32, value: i32) {
        let cmd = match setting {
            0 => AppCommand::SetTheme(match value { 1 => ThemeMode::Light, 2 => ThemeMode::Dark, _ => ThemeMode::System }),
            1 => AppCommand::SetQuality(match value { 0 => AudioQuality::High, 2 => AudioQuality::HiRes, _ => AudioQuality::Lossless }),
            2 => AppCommand::SetClose(if value == 0 { CloseAction::Exit } else { CloseAction::Tray }),
            3 => AppCommand::SetLyrics(value != 0), 6 => AppCommand::ToggleShuffle, _ => return,
        };
        self.snd(cmd);
    }

    /// 从有界图片缓存组装最多14张相邻播放列表封面的滑动窗口。
    fn synflw(&mut self, ui: &AppWindow) {
        let bg = if ui.global::<Theme>().get_dark() { [32, 39, 44] } else { [241, 235, 229] };
        if self.flwbg != bg {
            self.flwbg = bg;
            self.covers.stbg(bg);
        }
        if matches!(self.state.account, AccountState::Guest) {
            let qr_img = self.worker.qr_img.lock().unwrap().clone();
            let qr_changed = match (&self.qr_img, &qr_img) {
                (Some(a), Some(b)) => !Arc::ptr_eq(a, b),
                (None, None) => false,
                _ => true,
            };
            if qr_changed {
                self.qr_img = qr_img.clone();
                self.covers.stqr(qr_img);
            }
            if !self.flwslots.is_empty() || self.flwtotal != 0 {
                self.flwslots.clear();
                self.flwtotal = 0;
                self.flwtarget = 0;
                self.flwmiss = false;
                self.covers.stwin(Vec::new(), 0, 0);
            }
            return;
        }
        if self.qr_img.is_some() {
            self.qr_img = None;
            self.covers.stqr(None);
        }

        let (total, target, slots) = if !self.state.decks.is_empty() {
            let total = self.state.decks.len();
            let target = self.expected_cursor.map(|(c, _)| c).or(self.state.active_deck).unwrap_or(0).min(total - 1);
            let center = if self.covers.active() { self.covers.curidx().min(total - 1) } else { target };
            let mut indices: Vec<usize> = Vec::new();
            for base in [center, target] {
                for delta in -3isize..=3isize {
                    if let Some(idx) = base.checked_add_signed(delta) {
                        if idx < total && !indices.contains(&idx) { indices.push(idx); }
                    }
                }
            }
            indices.sort_unstable();
            let mut missing = false;
            let slots: Vec<(usize, Option<Arc<RgbaImage>>, Option<[u8; 3]>)> = indices.into_iter().map(|idx| {
                let dist = idx.abs_diff(target).min(idx.abs_diff(center));
                let size = if dist <= 1 { 320 } else { 80 };
                let deck = &self.state.decks[idx];
                // 活动卡播放时跟随当前封面。
                let now_cover = self.state.current.as_ref()
                    .filter(|_| self.state.active_deck == Some(idx) && self.state.playback != PlaybackState::Idle)
                    .and_then(|t| t.cover_key.as_deref());
                let url = now_cover.or(deck.cover_key.as_deref());
                let loaded = url.and_then(|u| self.worker.art.get(u, size)).map(|art| art.pixels);
                if loaded.is_none() && url.is_some_and(|u| !u.is_empty()) { missing = true; }
                let fallback = match deck.kind {
                    DeckKind::Like => self.like_art.clone(),
                    DeckKind::DailyRecommend => self.daily_art.clone(),
                    _ => self.fallback_art.clone(),
                };
                // 卡片环形标识用主题色。
                let ring = match deck.kind {
                    DeckKind::Like => Some(if bg[0] < 128 { [255, 129, 144] } else { [204, 70, 96] }),
                    DeckKind::DailyRecommend => Some([0, 181, 212]),
                    _ => None,
                };
                let img = loaded
                    .or_else(|| if size > 80 { url.and_then(|u| self.worker.art.peek(u, 80)).map(|art| art.pixels) } else { None })
                    .or(Some(fallback));
                (idx, img, ring)
            }).collect();
            self.flwmiss = missing;
            (total, target, slots)
        } else {
            self.flwmiss = false;
            (0, 0, Vec::new())
        };

        let trigger_login_flow = self.state.login_flow_seq != self.last_login_flow && total > 0;
        if trigger_login_flow {
            self.last_login_flow = self.state.login_flow_seq;
            self.flwslots = slots.clone();
            self.flwtotal = total;
            self.flwtarget = target;
            self.covers.flwlogin(slots, total, target);
            return;
        }
        let same_slots = self.flwslots.len() == slots.len() && self.flwslots.iter().zip(&slots).all(|((ia, a, ra), (ib, b, rb))| {
            ia == ib && ra == rb && match (a, b) { (Some(pa), Some(pb)) => Arc::ptr_eq(pa, pb), (None, None) => true, _ => false }
        });
        if !same_slots || self.flwtotal != total || self.flwtarget != target {
            self.covers.stwin(slots.clone(), total, target);
            self.flwslots = slots;
            self.flwtotal = total;
            self.flwtarget = target;
        }
    }

    /// 接收异步渲染线程产出的新帧并按动画状态动态调整刷新率。
    fn stepflw(&mut self, ui: &AppWindow) {
        let visible = !ui.window().is_minimized() && ui.window().is_visible() && ui.get_active_route() == 0;
        self.covers.stvis(visible);
        if !visible {
            if self.fast_timer {
                self.timer.set_interval(Duration::from_millis(100));
                self.fast_timer = false;
            }
            return;
        }
        if self.flwmiss || self.covers.active() {
            self.synflw(ui);
        }
        let next = self.covers.pllfrm();
        if frmchg(&self.frame, &next) {
            ui.set_coverflow_image(frmimg(&next));
            self.frame = next;
        }
        let want_fast = self.covers.active() || self.wheel_commit.is_some();
        if want_fast != self.fast_timer {
            self.timer.set_interval(Duration::from_millis(if want_fast { 16 } else { 100 }));
            self.fast_timer = want_fast;
        }
    }

    /// 同步轻量快照，只有列表变化时替换模型。
    pub fn tick(&mut self) {
        let Some(ui) = self.ui.upgrade() else { return; };
        if !self.hooked && crate::native::drgwin(ui.window(), false).is_ok() {
            self.hooked = true;
            crate::native::wndicn(ui.window());
            let dark = ui.global::<Theme>().get_dark();
            self.dark = Some(dark);
            crate::native::stthm(ui.window(), dark);
        }
        let scale = ui.window().scale_factor();
        if scale.is_finite() && scale > 0. && (scale - self.scale).abs() > 0.001 {
            self.scale = scale;
            ui.window().set_size(LogicalSize::new(360., 480.));
            self.covers.rsz((360. * scale).ceil() as u32, (274. * scale).ceil() as u32);
            crate::native::synwnd(ui.window());
        }
        if let Some((idx, when)) = self.wheel_commit {
            if Instant::now() >= when {
                self.wheel_commit = None;
                if self.state.active_deck != Some(idx) {
                    self.expected_cursor = Some((idx, Instant::now()));
                    self.snd(AppCommand::SelectDeck { index: idx, auto_play: true });
                }
            }
        }
        if self.synced.elapsed() >= Duration::from_millis(250) {
            self.synced = Instant::now();
            if let Some(tray) = &self.tray { match tray.pllevt() { 1 => crate::native::vsbl(ui.window(),true), 2 => { let _ = quit_event_loop(); }, _ => () } }
            let state = self.worker.state.lock().unwrap().clone();
            if self.state.queue != state.queue {
                ui.set_queue_tracks(trkmdl(&state.queue));
            }
            if let Some((exp, started)) = self.expected_cursor {
                if self.wheel_commit.is_none() && (state.active_deck == Some(exp) || state.error.is_some() || started.elapsed() > Duration::from_millis(2500)) {
                    self.expected_cursor = None;
                }
            }
            if self.state.search_results != state.search_results { ui.set_search_tracks(trkmdl(&state.search_results)); }
            if self.state.album_view != state.album_view {
                ui.set_album_open(state.album_view.is_some());
                ui.set_album_title(state.album_view.as_ref().map(|a| a.title.as_str().into()).unwrap_or_default());
                ui.set_album_tracks(trkmdl(state.album_view.as_ref().map(|a| a.tracks.as_slice()).unwrap_or(&[])));
            }
            if self.state.notice != state.notice {
                if let Some(text) = &state.notice { self.ntfy(text.clone()); }
            }
            if self.state.user_playlists != state.user_playlists {
                ui.set_user_playlists(plmdl(&state.user_playlists));
                if state.add_to_prompt.is_some() {
                    ui.set_add_playlists(addplmdl(&state.user_playlists, &self.add_playlist_selected));
                }
            }
            if self.state.opened_playlist != state.opened_playlist {
                ui.set_playlist_opened(state.opened_playlist.is_some());
                if let Some(ref opened) = state.opened_playlist {
                    ui.set_opened_playlist_title(opened.meta.name.as_str().into());
                    ui.set_opened_playlist_is_like(opened.meta.is_like);
                    ui.set_favorite_tracks(trkmdl(&opened.tracks));
                } else {
                    ui.set_favorite_tracks(trkmdl(&state.favorites));
                }
            } else if state.opened_playlist.is_none() && self.state.favorites != state.favorites {
                ui.set_favorite_tracks(trkmdl(&state.favorites));
            }
            let cur_deck = state.active_deck.and_then(|i| state.decks.get(i));
            ui.set_current_deck_removable(cur_deck.is_some_and(|d| d.removable));
            ui.set_current_deck_title(cur_deck.map(|d| d.title.as_str()).unwrap_or("").into());
            let has_col = cur_deck.is_some() || state.current.as_ref().is_some_and(|t| t.album_id.is_some());
            let is_alb = cur_deck.map(|d| matches!(d.kind, DeckKind::Album)).unwrap_or_else(|| state.current.as_ref().is_some_and(|t| t.album_id.is_some()));
            ui.set_current_has_collection(has_col);
            ui.set_current_collection_is_album(is_alb);

            let prompt_key = state.add_to_prompt.as_ref().map(|p| (p.source_id.clone(), p.whole_album));
            if prompt_key != self.add_prompt_key {
                self.add_prompt_key = prompt_key;
                self.add_playlist_selected.clear();
                ui.set_add_sel_flow(false);
                ui.set_add_sel_like(false);
                ui.set_creating_playlist(false);
                ui.set_add_playlists(addplmdl(&state.user_playlists, &self.add_playlist_selected));
            }
            ui.set_add_modal_visible(state.add_to_prompt.is_some() || self.pick_pending.is_some());
            ui.set_add_modal_title(state.add_to_prompt.as_ref().map(|p| if p.whole_album { format!("专辑 · {}", p.title) } else { p.title.clone() })
                .or_else(|| self.pick_pending.map(|(index, _)| self.state.search_results.get(index).map(|t| t.title.clone()).unwrap_or_default()))
                .unwrap_or_default().into());
            ui.set_confirm_unlike_text(state.confirm_unlike.as_deref().unwrap_or("").into());

            let dl_key = state.download_prompt.as_ref().map(|p| p.collection_name.clone());
            if dl_key != self.dl_prompt_key {
                self.dl_prompt_key = dl_key;
                if let Some(ref prompt) = state.download_prompt {
                    let def = state.settings.audio.quality;
                    let picked = if prompt.available.contains(&def) { def } else { prompt.available.first().copied().unwrap_or(AudioQuality::High) };
                    self.dl_selected_q = qtocode(picked);
                    ui.set_dl_selected_q(self.dl_selected_q);
                    ui.set_dl_qualities(qltmdl(&prompt.available, self.dl_selected_q));
                }
            }
            ui.set_dl_modal_visible(state.download_prompt.is_some());
            ui.set_dl_modal_title(state.download_prompt.as_ref().map(|p| format!("{} ({}首)", p.collection_name, p.tracks.len())).unwrap_or_default().into());

            match &state.account {
                AccountState::Guest => { ui.set_is_guest(true); ui.set_user_name("未登录".into()); ui.set_avatar_url("".into()); },
                AccountState::Authorized { display_name, avatar_ref, .. } => { ui.set_is_guest(false); ui.set_user_name(display_name.into()); ui.set_avatar_url(avatar_ref.as_deref().unwrap_or("").into()); },
            }
            ui.set_checkin_label(match state.checkin { CheckInState::Checked => "已签到", CheckInState::Pending => "签到中", CheckInState::Available => "签到" }.into());
            ui.set_track_title(state.current.as_ref().map(|t|t.title.as_str()).unwrap_or_else(|| cur_deck.map(|d| d.title.as_str()).unwrap_or("尚未播放")).into());
            ui.set_artist_name(state.current.as_ref().map(|t|t.artist_text.as_str()).unwrap_or("").into());
            ui.set_is_playing(state.playback == PlaybackState::Playing);
            ui.set_is_favorite(state.is_favorite);
            ui.set_repeat_mode(match state.repeat { RepeatMode::Off => 0, RepeatMode::All => 1, RepeatMode::One => 2 });
            ui.set_queue_cursor(state.queue_cursor.map(|i|i as i32).unwrap_or(-1));
            ui.set_position_ms(state.position_ms.min(i32::MAX as u64) as i32);
            ui.set_duration_ms(state.duration_ms.min(i32::MAX as u64) as i32);
            ui.set_volume(state.volume);
            ui.set_favorites_local(state.favorites_local);
            ui.set_busy(self.worker.busy.load(std::sync::atomic::Ordering::Acquire));
            let mode = match state.theme { ThemeMode::System => 0, ThemeMode::Light => 1, ThemeMode::Dark => 2 };
            ui.set_theme_idx(mode); ui.global::<Theme>().set_mode(mode);
            let dark = ui.global::<Theme>().get_dark();
            if self.hooked && self.dark != Some(dark) {
                self.dark = Some(dark);
                crate::native::stthm(ui.window(), dark);
            }
            ui.set_quality_idx(match state.settings.audio.quality { AudioQuality::Standard | AudioQuality::High => 0, AudioQuality::Lossless => 1, AudioQuality::HiRes => 2 });
            ui.set_close_tray(state.settings.window.close_action == CloseAction::Tray);
            ui.set_desktop_lyrics(state.settings.lyrics.desktop_enabled);
            ui.set_shuffle(state.shuffle);
            ui.set_cache_dir(if state.settings.cache.directory.as_os_str().is_empty() { "默认缓存".into() } else { state.settings.cache.directory.display().to_string().into() });
            ui.set_download_dir(if state.settings.downloads.directory.as_os_str().is_empty() { "系统下载目录".into() } else { state.settings.downloads.directory.display().to_string().into() });
            let qr = self.worker.qr.lock().unwrap().clone();
            if frmchg(&self.qr,&qr) { ui.set_qr_image(frmimg(&qr)); self.qr = qr; }
            ui.set_qr_label(state.login_qr.as_ref().map(|qr|match qr.stage { QrStage::Waiting => "用酷狗概念版扫码", QrStage::Confirming => "请在手机上确认", QrStage::Authorized => "登录成功", QrStage::Expired => "二维码已过期，点击刷新" }).unwrap_or("").into());
            if state.settings.lyrics.desktop_enabled {
                if self.lyrics.is_none() {
                    match LyricWindow::new() {
                        Ok(lyrics) => { let _ = lyrics.show(); self.lyrics = Some(lyrics); self.lyrics_hooked = false; }
                        Err(error) => self.ntfy(error.to_string()),
                    }
                }
                if let Some(lyrics) = &self.lyrics {
                    lyrics.set_line(state.lyric_text.as_str().into());
                    // 句柄就绪前逐帧重试。
                    if !self.lyrics_hooked { self.lyrics_hooked = crate::native::pnwin(lyrics.window()); }
                }
            } else {
                // 关闭时显式销毁窗口。
                if let Some(lyrics) = self.lyrics.take() {
                    let _ = lyrics.hide();
                    crate::native::rmlwin(lyrics.window());
                }
                self.lyrics_hooked = false;
            }
            if state.error != self.state.error { if let Some(error) = &state.error { self.ntfy(error.clone()); } }
            if state.download != self.state.download {
                if let Some(job) = &state.download {
                    let prefix = if job.total > 1 { format!("({}/{}) ", (job.completed + 1).min(job.total), job.total) } else { String::new() };
                    self.ntfy(match &job.state {
                        DownloadState::Queued => format!("{prefix}准备下载…"),
                        DownloadState::Downloading { received, total: Some(total) } if *total > 0 => format!("{prefix}下载中{}%",received.saturating_mul(100)/total),
                        DownloadState::Downloading { received, .. } => format!("{prefix}下载中{:.1}MB",*received as f64/1_000_000.),
                        DownloadState::Completed => format!("已下载：{}",job.dest_path),
                        DownloadState::Failed(error) => format!("下载失败：{error}"),
                    });
                }
            }
            if state.checkin != self.state.checkin && state.checkin == CheckInState::Checked { self.ntfy("今日签到完成".into()); }
            ui.set_notice(if self.noted.elapsed() < Duration::from_secs(6) { self.note.as_str() } else { "" }.into());
            ui.global::<MediaArt>().set_epoch(ui.global::<MediaArt>().get_epoch().wrapping_add(1));
            drop(self.worker.art.rtrd());
            if state.deck_flow_seq != self.last_deck_flow {
                self.last_deck_flow = state.deck_flow_seq;
                self.expected_cursor = state.active_deck.map(|i| (i, Instant::now()));
            }
            self.state = state;
            self.synflw(&ui);
        }
        self.stepflw(&ui);
    }
}

/// 转换元数据而不加载列表图片。
fn trkmdl(tracks: &[TrackSummary]) -> ModelRc<TrackItem> {
    ModelRc::new(VecModel::from(tracks.iter().map(|t| TrackItem {
        title: t.title.as_str().into(),
        artist: t.artist_text.as_str().into(),
        cover: t.cover_key.as_deref().unwrap_or("").into(),
        duration: t.duration_ms.map(|ms|format!("{:02}:{:02}",ms/60_000,ms/1000%60)).unwrap_or_default().into(),
        has_album: t.album_id.as_deref().is_some_and(|s| !s.is_empty()),
    }).collect::<Vec<_>>()))
}

/// 转换用户歌单摘要为收藏页列表模型。
fn plmdl(lists: &[UserPlaylistMeta]) -> ModelRc<PlaylistItem> {
    ModelRc::new(VecModel::from(lists.iter().map(|p| PlaylistItem {
        id: p.id.as_str().into(),
        name: p.name.as_str().into(),
        cover: p.cover_key.as_deref().unwrap_or("").into(),
        count: format!("{}首", p.song_count).into(),
        is_like: p.is_like,
    }).collect::<Vec<_>>()))
}

/// 转换可写歌单为「添加至歌单」勾选模型。
fn addplmdl(lists: &[UserPlaylistMeta], selected: &[String]) -> ModelRc<AddPlaylistItem> {
    ModelRc::new(VecModel::from(lists.iter().filter(|p| p.is_own && !p.is_like).map(|p| AddPlaylistItem {
        name: p.name.as_str().into(),
        selected: selected.iter().any(|id| id == &p.id),
    }).collect::<Vec<_>>()))
}

/// 转换上游可用下载品质为选项模型。
fn qltmdl(qualities: &[AudioQuality], selected: i32) -> ModelRc<QualityItem> {
    ModelRc::new(VecModel::from(qualities.iter().map(|q| {
        let code = qtocode(*q);
        QualityItem { code, label: q.qltlbl().into(), selected: code == selected }
    }).collect::<Vec<_>>()))
}

/// 音质枚举与界面整型代码转换。
fn qtocode(q: AudioQuality) -> i32 {
    match q { AudioQuality::Standard => 0, AudioQuality::High => 1, AudioQuality::Lossless => 2, AudioQuality::HiRes => 3 }
}

/// 界面整型代码与音质枚举转换。
fn codetoq(code: i32) -> AudioQuality {
    match code { 0 => AudioQuality::Standard, 2 => AudioQuality::Lossless, 3 => AudioQuality::HiRes, _ => AudioQuality::High }
}

/// 按共享缓冲身份判断图像变化。
fn frmchg(before: &Option<Arc<Frame>>, after: &Option<Arc<Frame>>) -> bool {
    match (before,after) { (Some(a),Some(b)) => !Arc::ptr_eq(a,b), (None,None) => false, _ => true }
}

/// 将共享像素缓冲转换为界面图像。
fn frameimg(frame: &Frame) -> Image { Image::from_rgba8(frame.clone()) }

/// 共享现有RGBA缓冲，不复制像素。
fn frmimg(frame: &Option<Arc<Frame>>) -> Image { frame.as_ref().map(|f| frameimg(f)).unwrap_or_default() }

#[cfg(test)]
mod tests {
    use super::*;
    /// 同一缓冲不重建，换图和清空必须更新。
    #[test]
    fn tst_frm() {
        let a = Some(Arc::new(Frame::new(1,1)));
        assert!(!frmchg(&a,&a.clone())); assert!(frmchg(&a,&None));
        assert!(frmchg(&a,&Some(Arc::new(Frame::new(1,1)))));
    }

    /// 排版缓存淘汰后，等宽新歌名和总时长仍须刷新。
    #[test]
    fn tst_txtcache() {
        struct Platform(Rc<plat::MinimalSoftwareWindow>);
        impl plat::Platform for Platform {
            /// 提供无系统窗口的软件渲染器。
            fn create_window_adapter(&self) -> Result<Rc<dyn plat::WindowAdapter>, PlatformError> { Ok(self.0.clone()) }
        }
        let window = plat::MinimalSoftwareWindow::new(plat::RepaintBufferType::NewBuffer);
        plat::set_platform(Box::new(Platform(window.clone()))).unwrap();
        let ui = AppWindow::new().unwrap();
        window.set_size(PhysicalSize::new(360,480));
        ui.set_is_guest(false);
        ui.set_track_title("万人邂逅".into()); ui.set_duration_ms(298_000);
        ui.set_volume(0.5);
        ui.global::<Theme>().set_mode(1);
        ui.show().unwrap();
        let mut pixels = SharedPixelBuffer::<Rgb8Pixel>::new(360,480);
        assert!(window.draw_if_needed(|r| { r.render(pixels.make_mut_slice(),360); }));
        for mode in [2,1] {
            ui.global::<Theme>().set_mode(mode);
            assert!(window.draw_if_needed(|r| { r.set_repaint_buffer_type(plat::RepaintBufferType::ReusedBuffer); r.render(pixels.make_mut_slice(),360); r.text_layout_cache().clear_all(); }));
        }
        // 截取两处文本像素。
        let crop = |frame: &SharedPixelBuffer<Rgb8Pixel>, x: usize, y: usize, w: usize, h: usize| {
            (y..y+h).flat_map(|row| frame.as_bytes()[3*(row*360+x)..3*(row*360+x+w)].iter().copied()).collect::<Vec<_>>()
        };
        let title = crop(&pixels,24,316,312,22);
        let duration = crop(&pixels,318,341,30,22);
        assert!(crop(&pixels,64,453,1,1).iter().zip([241u8,235,229]).all(|(a,b)| a.abs_diff(b) < 32),"volume fill lost its rounded end");
        assert_ne!(crop(&pixels,80,453,1,1),[241,235,229],"volume fill disappeared");
        ui.set_track_title("三年二班".into()); ui.set_duration_ms(280_000);
        assert!(window.draw_if_needed(|r| { r.render(pixels.make_mut_slice(),360); }));
        assert!(title != crop(&pixels,24,316,312,22),"song title pixels stayed stale");
        assert!(duration != crop(&pixels,318,341,30,22),"duration pixels stayed stale");
        assert_eq!(crop(&pixels,179,9,1,1),[220,213,205],"header is not anchored to the top");
        // 未选中标签按序叠放。
        assert_eq!(crop(&pixels,198,35,1,1),[230,223,215],"search tab did not overlap favorites tab");
        assert_eq!(crop(&pixels,250,35,1,1),[230,223,215],"favorites tab did not overlap settings tab");
        ui.set_active_route(2);
        window.draw_if_needed(|r| { r.render(pixels.make_mut_slice(),360); });
        std::thread::sleep(Duration::from_millis(220));
        plat::update_timers_and_animations();
        window.draw_if_needed(|r| { r.render(pixels.make_mut_slice(),360); });
        assert_eq!(crop(&pixels,179,9,1,1),[241,235,229],"selected tab did not rise above its neighbors");
        assert_eq!(crop(&pixels,179,41,1,1),crop(&pixels,179,45,1,1),"tab and content have a seam");
        assert_eq!(crop(&pixels,146,30,1,1),[241,235,229],"selected search tab did not overlap account tab");
        assert_eq!(crop(&pixels,198,35,1,1),[241,235,229],"selected search tab did not overlap favorites tab");
        ui.set_active_route(3);
        window.draw_if_needed(|r| { r.render(pixels.make_mut_slice(),360); });
        std::thread::sleep(Duration::from_millis(220));
        plat::update_timers_and_animations();
        window.draw_if_needed(|r| { r.render(pixels.make_mut_slice(),360); });
        assert_eq!(crop(&pixels,198,35,1,1),[241,235,229],"selected favorites tab did not overlap search tab");
        assert_eq!(crop(&pixels,250,35,1,1),[241,235,229],"selected favorites tab did not overlap settings tab");
        ui.set_active_route(4);
        window.draw_if_needed(|r| { r.render(pixels.make_mut_slice(),360); });
        std::thread::sleep(Duration::from_millis(220));
        plat::update_timers_and_animations();
        window.draw_if_needed(|r| { r.render(pixels.make_mut_slice(),360); });
        assert_eq!(crop(&pixels,198,35,1,1),[230,223,215],"search tab lost overlap over favorites when settings selected");
        assert_eq!(crop(&pixels,250,35,1,1),[241,235,229],"selected settings tab did not overlap favorites tab");
        // 核验音质分段高亮。
        for (qidx, active_x) in [(0usize, 184usize), (1, 245), (2, 306)] {
            ui.set_quality_idx(qidx as i32);
            window.draw_if_needed(|r| { r.render(pixels.make_mut_slice(),360); });
            assert_eq!(crop(&pixels, active_x, 173, 1, 1), [0, 170, 199], "quality pill {qidx} did not highlight");
        }

        // 核验新增界面可渲染。
        ui.set_search_tracks(trkmdl(&[TrackSummary::nw(TrackId::kg("t1"), "迟来的人", "蔡琴")]));
        ui.set_active_route(2);
        ui.set_add_choice_idx(0);
        ui.set_add_choice_has_album(true);
        assert!(window.draw_if_needed(|r| { r.render(pixels.make_mut_slice(),360); }));
        ui.set_add_choice_idx(-1);
        ui.set_add_modal_title("专辑 · 你不要那样看着我的眼睛".into());
        ui.set_add_modal_visible(true);
        ui.set_creating_playlist(true);
        assert!(window.draw_if_needed(|r| { r.render(pixels.make_mut_slice(),360); }));
        ui.set_add_modal_visible(false);
        ui.set_user_playlists(plmdl(&[UserPlaylistMeta {
            id: "1".into(), list_id: Some(1), global_id: None, name: "我喜欢".into(),
            cover_key: None, song_count: 1, is_own: true, is_like: true,
        }]));
        ui.set_active_route(3);
        assert!(window.draw_if_needed(|r| { r.render(pixels.make_mut_slice(),360); }));
        ui.set_playlist_opened(true);
        ui.set_opened_playlist_title("蔡琴的经典".into());
        assert!(window.draw_if_needed(|r| { r.render(pixels.make_mut_slice(),360); }));
        ui.set_confirm_unlike_text("确认从歌单“我喜欢”中删除这首歌吗？".into());
        assert!(window.draw_if_needed(|r| { r.render(pixels.make_mut_slice(),360); }));
        ui.set_confirm_unlike_text("".into());
        ui.set_dl_modal_visible(true);
        ui.set_dl_qualities(qltmdl(&[AudioQuality::High, AudioQuality::Lossless, AudioQuality::HiRes], 2));
        assert!(window.draw_if_needed(|r| { r.render(pixels.make_mut_slice(),360); }));
    }

    }
