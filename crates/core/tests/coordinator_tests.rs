use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use kugou_core::application::commands::AppCommand;
use kugou_core::application::coordinator::Coordinator;
use kugou_core::application::ports::{PlaybackPort, StoragePort, LibraryPort};
use kugou_core::domain::error::{AppError, PlaybackError};
use kugou_core::domain::playback::{PlaybackProgress, PlaybackState, RepeatMode};
use kugou_core::domain::settings::ThemeMode;
use kugou_core::domain::track::{ResolvedSource, TrackSummary};
use kugou_core::domain::ids::TrackId;
use kugou_core::provider::mock::MockProvider;
use kugou_core::storage::database::DatabaseStorage;

/// 记录核心用例触发的播放控制。
struct TestPlayer {
    progress: Mutex<PlaybackProgress>,
    fail: AtomicBool,
    plays: AtomicUsize,
}

impl TestPlayer {
    /// 创建无需音频设备的测试播放器。
    fn nw() -> Self {
        Self { progress: Mutex::new(PlaybackProgress { position_ms: 0, duration_ms: 210000, volume: 0.5, state: PlaybackState::Idle }), fail: AtomicBool::new(false), plays: AtomicUsize::new(0) }
    }
}

impl PlaybackPort for TestPlayer {
    /// 记录一次播放或注入设备失败。
    fn ply(&self, _source: ResolvedSource) -> Result<(), AppError> {
        if self.fail.load(Ordering::Relaxed) { return Err(PlaybackError::Device("test".into()).into()); }
        let mut progress = self.progress.lock().unwrap();
        progress.state = PlaybackState::Playing;
        progress.position_ms = 0;
        self.plays.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
    /// 记录暂停。
    fn ps(&self) { self.progress.lock().unwrap().state = PlaybackState::Paused; }
    /// 记录继续。
    fn rsm(&self) { self.progress.lock().unwrap().state = PlaybackState::Playing; }
    /// 记录停止。
    fn stp(&self) { self.progress.lock().unwrap().state = PlaybackState::Idle; }
    /// 记录定位。
    fn sk(&self, position: u64) { self.progress.lock().unwrap().position_ms = position; }
    /// 记录音量。
    fn stvlm(&self, volume: f32) { self.progress.lock().unwrap().volume = volume; }
    /// 返回测试进度。
    fn prgrss(&self) -> PlaybackProgress { self.progress.lock().unwrap().clone() }
}

/// 组装共享同一存储的测试用例。
fn coord(storage: Arc<DatabaseStorage>, player: Arc<TestPlayer>) -> Coordinator {
    let provider = Arc::new(MockProvider::nw());
    Coordinator::nw(provider.clone(), provider.clone(), provider.clone(), provider, player, storage)
}

/// 验证搜索、队列、收藏、设置恢复和失败后的状态一致性。
#[test]
fn tst_coreflow() {
    let storage = Arc::new(DatabaseStorage::inmem().unwrap());
    let player = Arc::new(TestPlayer::nw());
    let mut app = coord(storage.clone(), player.clone());
    app.hndlcmd(AppCommand::Search(" music ".into()));
    let first = app.ssht().search_results[0].clone();
    let second = app.ssht().search_results[1].clone();
    
    app.hndlcmd(AppCommand::QueueAdd(first.id.clone()));
    app.hndlcmd(AppCommand::QueueAdd(second.id.clone()));
    app.hndlcmd(AppCommand::QueueAdd(first.id.clone()));
    app.hndlcmd(AppCommand::PlayTrack(first.id.clone()));
    assert_eq!(app.ssht().current, Some(first.clone()));
    assert_eq!(app.ssht().queue.len(), 3);
    app.hndlcmd(AppCommand::Next);
    assert_eq!(app.queue.cursor, Some(1));
    app.hndlcmd(AppCommand::Next);
    assert_eq!(app.queue.cursor, Some(2));
    app.hndlcmd(AppCommand::ToggleFavorite(first.id.clone()));
    assert!(app.ssht().is_favorite);
    app.hndlcmd(AppCommand::SetVolume(2.0));
    app.hndlcmd(AppCommand::SetTheme(ThemeMode::Dark));
    app.hndlcmd(AppCommand::SetRepeat(RepeatMode::Off));
    assert_eq!(storage.ldcfg().unwrap().audio.volume, 1.0);
    app.hndlcmd(AppCommand::SetVolume(f32::NAN));
    assert!(app.ssht().error.is_some());
    assert_eq!(storage.ldcfg().unwrap().audio.volume, 1.0);
    let mut restored = coord(storage.clone(), Arc::new(TestPlayer::nw()));
    assert_eq!(restored.queue.cursor, Some(2));
    assert_eq!(restored.ssht().queue, app.ssht().queue);
    assert_eq!(restored.ssht().favorites, vec![first.clone()]);
    assert_eq!(restored.ssht().theme, ThemeMode::Dark);
    restored.hndlcmd(AppCommand::ToggleFavorite(first.id.clone()));
    assert!(restored.ssht().confirm_unlike.is_some());
    restored.hndlcmd(AppCommand::ConfirmUnlike(true));
    assert!(storage.ldfvrt().unwrap().is_empty());
    app.hndlcmd(AppCommand::PlayTrack(first.id.clone()));
    player.fail.store(true, Ordering::Relaxed);
    app.hndlcmd(AppCommand::Prev);
    assert!(app.ssht().error.is_some());
    assert_eq!(app.queue.cursor, Some(2));
    assert_eq!(app.ssht().current, Some(first));
    player.fail.store(false, Ordering::Relaxed);
    app.hndlcmd(AppCommand::SetRepeat(RepeatMode::All));
    player.progress.lock().unwrap().state = PlaybackState::Ended;
    app.tick();
    let plays = player.plays.load(Ordering::Relaxed);
    app.tick();
    assert_eq!(player.plays.load(Ordering::Relaxed), plays);
    app.hndlcmd(AppCommand::QueueClear);
    assert!(storage.ldqu().unwrap().0.is_empty());
    assert!(app.ssht().current.is_none());
}

/// 模拟官方收藏写入后一次读取仍返回旧列表。
struct LagLibrary { track: TrackSummary, reads: AtomicUsize }

impl LibraryPort for LagLibrary {
    /// 首次返回旧数据，随后返回已确认收藏。
    fn fvrts(&self) -> Result<Vec<TrackSummary>, AppError> {
        Ok(if self.reads.fetch_add(1, Ordering::Relaxed) == 0 { Vec::new() } else { vec![self.track.clone()] })
    }
    /// 模拟已经接受写入。
    fn tglfvrt(&self, _: &TrackSummary) -> Result<bool, AppError> { Ok(true) }
    /// 返回模拟结果。
    fn isfvrt(&self, _: &TrackId) -> Result<bool, AppError> { Ok(true) }
}

/// 验证云端延迟可见不会被误判为操作失败。
#[test]
fn tst_cldlag() {
    let mut app = coord(Arc::new(DatabaseStorage::inmem().unwrap()), Arc::new(TestPlayer::nw()));
    app.hndlcmd(AppCommand::Search("test".into()));
    let track = app.ssht().search_results[0].clone();
    app.hndlcmd(AppCommand::PlayTrack(track.id.clone()));
    let library = Arc::new(LagLibrary { track: track.clone(), reads: AtomicUsize::new(0) });
    app.library = library.clone();
    app.snapshot.favorites_local = false;
    app.hndlcmd(AppCommand::ToggleFavorite(track.id));
    assert!(app.ssht().error.is_none());
    assert!(app.ssht().is_favorite);
    assert_eq!(library.reads.load(Ordering::Relaxed), 2);
}

/// 按已核实的完整专辑、Single 与歌单检查曲序和持久化边界。
#[test]
fn tst_colls() {
    let cohorts: kugou_core::deps::serde_json::Value = kugou_core::deps::serde_json::from_str(include_str!("fixtures/collections.json")).unwrap();
    for cohort in cohorts.as_array().unwrap() {
        let tracks: Vec<TrackSummary> = kugou_core::deps::serde_json::from_value(cohort["tracks"].clone()).unwrap();
        let storage = Arc::new(DatabaseStorage::inmem().unwrap());
        storage.svtrks(&tracks).unwrap();
        let player = Arc::new(TestPlayer::nw());
        let mut app = coord(storage.clone(), player.clone());
        app.hndlcmd(AppCommand::SetRepeat(RepeatMode::Off));
        for track in &tracks { app.hndlcmd(AppCommand::QueueAdd(track.id.clone())); }
        app.hndlcmd(AppCommand::QueuePlay(0));
        for (index, track) in tracks.iter().enumerate() {
            assert_eq!(app.ssht().current.as_ref(), Some(track), "{} index {index}", cohort["name"]);
            assert_eq!(app.ssht().queue_cursor, Some(index));
            let mut progress = player.progress.lock().unwrap();
            progress.position_ms = progress.duration_ms;
            progress.state = PlaybackState::Ended;
            drop(progress);
            app.tick();
            if index + 1 < tracks.len() { assert_eq!(app.ssht().position_ms, 0); }
        }
        assert_eq!(player.plays.load(Ordering::Relaxed), tracks.len());
        assert_eq!(app.ssht().playback, PlaybackState::Idle);
        app.tick();
        assert_eq!(player.plays.load(Ordering::Relaxed), tracks.len());
        let restored = coord(storage.clone(), Arc::new(TestPlayer::nw()));
        assert_eq!(restored.ssht().queue, tracks);
        assert_eq!(restored.ssht().queue_cursor, Some(tracks.len() - 1));
        assert_eq!(restored.ssht().current.as_ref(), tracks.last());
        assert_eq!(restored.ssht().playback, PlaybackState::Idle);
        app.hndlcmd(AppCommand::SetRepeat(RepeatMode::All));
        player.progress.lock().unwrap().state = PlaybackState::Ended;
        app.tick();
        assert_eq!(app.ssht().queue_cursor, Some(0));
        app.hndlcmd(AppCommand::SetRepeat(RepeatMode::One));
        app.hndlcmd(AppCommand::SetLyrics(true));
        app.stlyrc(&tracks[0].id, vec![kugou_core::domain::track::LyricLine { begin_ms: 0, end_ms: None, text: "test lyric".into() }]);
        player.progress.lock().unwrap().state = PlaybackState::Ended;
        app.tick();
        assert_eq!(app.ssht().queue_cursor, Some(0));
        assert_eq!(app.ssht().lyric_text, "test lyric");
        app.hndlcmd(AppCommand::SetRepeat(RepeatMode::All));
        app.hndlcmd(AppCommand::ToggleShuffle);
        app.hndlcmd(AppCommand::Next);
        if tracks.len() == 1 { assert_eq!(app.ssht().queue_cursor, Some(0)); }
        else { assert_ne!(app.ssht().queue_cursor, Some(0)); }
        app.hndlcmd(AppCommand::QueueAdd(tracks[0].id.clone()));
        app.hndlcmd(AppCommand::QueuePlay(tracks.len()));
        assert_eq!(app.ssht().queue_cursor, Some(tracks.len()));
        app.hndlcmd(AppCommand::QueueRemove(0));
        assert_eq!(app.ssht().queue_cursor, Some(tracks.len() - 1));
        app.hndlcmd(AppCommand::QueueClear);
        for command in [AppCommand::Next, AppCommand::Prev, AppCommand::TogglePlay] { app.hndlcmd(command); assert!(app.ssht().error.is_none()); }
        assert!(app.ssht().current.is_none());
        assert!(app.ssht().queue.is_empty());
    }
}

/// 验证播放列表式 CoverFlow 全链路 12 步用户情景、批量分目录下载与本地无损优先播放。
#[test]
fn tst_scn12() {
    use kugou_core::application::commands::TargetScope;
    use kugou_core::domain::track::{AudioQuality, DeckKind, DownloadedFile};
    use kugou_core::application::ports::StreamPort;

    struct CountStream { hits: Arc<AtomicUsize>, fail: Arc<AtomicBool>, inner: MockProvider }
    impl StreamPort for CountStream {
        /// 统计按标识解析音源的调用次数。
        fn rslv(&self, id: &TrackId, qlt: AudioQuality) -> Result<ResolvedSource, AppError> {
            self.hits.fetch_add(1, Ordering::Relaxed);
            if self.fail.load(Ordering::Relaxed) {
                return Err(kugou_core::domain::error::NetworkError::Offline.into());
            }
            self.inner.rslv(id, qlt)
        }
        /// 统计按曲目摘要解析音源的调用次数。
        fn rslvtrk(&self, trk: &TrackSummary, qlt: AudioQuality) -> Result<ResolvedSource, AppError> {
            self.hits.fetch_add(1, Ordering::Relaxed);
            if self.fail.load(Ordering::Relaxed) {
                return Err(kugou_core::domain::error::NetworkError::Offline.into());
            }
            self.inner.rslvtrk(trk, qlt)
        }
    }

    let storage = Arc::new(DatabaseStorage::inmem().unwrap());
    let player = Arc::new(TestPlayer::nw());
    let provider = Arc::new(MockProvider::nw());
    let stream_hits = Arc::new(AtomicUsize::new(0));
    let stream_fail = Arc::new(AtomicBool::new(false));
    let stream = Arc::new(CountStream { hits: stream_hits.clone(), fail: stream_fail.clone(), inner: MockProvider::nw() });
    let mut app = Coordinator::nw(provider.clone(), provider.clone(), provider.clone(), stream, player.clone(), storage.clone());

    // 登录后停日推并播放。
    app.hndlcmd(AppCommand::LoadCoverFlow);
    assert_eq!(app.ssht().active_deck, Some(1));
    assert_eq!(app.ssht().decks[0].kind, DeckKind::Like);
    assert_eq!(app.ssht().decks[1].kind, DeckKind::DailyRecommend);
    assert_eq!(app.ssht().decks[2].kind, DeckKind::RecommendedPlaylist);
    app.hndlcmd(AppCommand::TogglePlay);
    assert_eq!(app.ssht().current.as_ref().map(|t| t.title.as_str()), Some("大城小爱"));

    // 搜索并入歌单与Flow。
    app.hndlcmd(AppCommand::Search("迟来的人".into()));
    app.hndlcmd(AppCommand::OpenAddTo { search_index: 0, whole_album: true });
    let prompt = app.ssht().add_to_prompt.clone().unwrap();
    assert_eq!(prompt.title, "你不要那样看着我的眼睛");
    assert_eq!(prompt.tracks.len(), 3);
    app.hndlcmd(AppCommand::CreatePlaylist("蔡琴的经典".into()));
    let new_id = app.ssht().add_to_prompt.as_ref().unwrap().created_playlist_id.clone().unwrap();
    app.hndlcmd(AppCommand::ConfirmAddTo {
        to_coverflow: true,
        to_like: true,
        playlist_ids: vec![new_id.clone()],
    });
    assert_eq!(app.ssht().decks[1].kind, DeckKind::DailyRecommend);
    assert_eq!(app.ssht().decks[2].kind, DeckKind::Album);
    assert_eq!(app.ssht().decks[2].title, "你不要那样看着我的眼睛");
    assert!(app.ssht().decks[2].removable);
    assert_eq!(app.ssht().decks[3].kind, DeckKind::RecommendedPlaylist);
    assert!(app.ssht().favorites.iter().any(|t| t.title == "雨中城市"));
    assert!(app.ssht().user_playlists.iter().any(|p| p.name == "蔡琴的经典" && p.song_count == 3));

    // 切专辑曲并取消删歌。
    app.hndlcmd(AppCommand::SelectDeck { index: 2, auto_play: true });
    assert_eq!(app.ssht().current.as_ref().map(|t| t.title.as_str()), Some("雨中城市"));
    assert!(app.ssht().is_favorite);
    app.hndlcmd(AppCommand::HeartClick);
    assert!(app.ssht().confirm_unlike.is_some());
    app.hndlcmd(AppCommand::ConfirmUnlike(false));
    assert!(app.ssht().confirm_unlike.is_none());
    assert!(app.ssht().is_favorite);

    // 循环模式跨Cover切换。
    app.hndlcmd(AppCommand::Next);
    assert_eq!(app.ssht().current.as_ref().map(|t| t.title.as_str()), Some("迟来的人"));
    app.hndlcmd(AppCommand::SetRepeat(RepeatMode::All));
    app.hndlcmd(AppCommand::QueuePlay(2));
    assert_eq!(app.ssht().current.as_ref().map(|t| t.title.as_str()), Some("你不要那样看着我的眼睛"));
    player.progress.lock().unwrap().state = PlaybackState::Ended;
    app.tick();
    assert_eq!(app.ssht().active_deck, Some(2));
    assert_eq!(app.ssht().queue_cursor, Some(0));

    app.hndlcmd(AppCommand::QueuePlay(2));
    app.hndlcmd(AppCommand::SetRepeat(RepeatMode::One));
    player.progress.lock().unwrap().state = PlaybackState::Ended;
    app.tick();
    assert_eq!(app.ssht().queue_cursor, Some(2));

    app.hndlcmd(AppCommand::SetRepeat(RepeatMode::Off));
    player.progress.lock().unwrap().state = PlaybackState::Ended;
    app.tick();
    assert_eq!(app.ssht().active_deck, Some(3));
    assert_eq!(app.ssht().queue_cursor, Some(0));

    // 本地无损优先播放。
    app.hndlcmd(AppCommand::SelectDeck { index: 2, auto_play: false });
    app.hndlcmd(AppCommand::InspectDownload { scope: TargetScope::Collection, search_index: None, whole_album: true });
    assert_eq!(app.ssht().download_prompt.as_ref().unwrap().available, vec![AudioQuality::High, AudioQuality::Lossless, AudioQuality::HiRes]);
    app.hndlcmd(AppCommand::CancelDownloadPrompt);

    let tmp_flac = std::env::temp_dir().join(format!("kodog-local-priority-{}.flac", std::process::id()));
    std::fs::write(&tmp_flac, b"fLaC").unwrap();
    storage.svdwnld(&DownloadedFile {
        track_id: TrackId::kg("alb_1"),
        quality: AudioQuality::Lossless,
        is_lossless: true,
        collection_name: "你不要那样看着我的眼睛".into(),
        file_path: tmp_flac.display().to_string(),
    }).unwrap();
    let before = stream_hits.load(Ordering::Relaxed);
    app.hndlcmd(AppCommand::QueuePlay(0));
    assert_eq!(stream_hits.load(Ordering::Relaxed), before, "本地无损文件应直接播放，不得请求在线流地址");
    std::fs::remove_file(tmp_flac).unwrap();

    // 在线失败回退本地MP3。
    let tmp_mp3 = std::env::temp_dir().join(format!("kodog-local-fallback-{}.mp3", std::process::id()));
    std::fs::write(&tmp_mp3, b"ID3").unwrap();
    storage.svdwnld(&DownloadedFile {
        track_id: TrackId::kg("alb_2"),
        quality: AudioQuality::High,
        is_lossless: false,
        collection_name: "你不要那样看着我的眼睛".into(),
        file_path: tmp_mp3.display().to_string(),
    }).unwrap();
    stream_fail.store(true, Ordering::Relaxed);
    app.hndlcmd(AppCommand::QueuePlay(1));
    assert!(app.ssht().error.is_none(), "断网且存在本地MP3时应兜底播本地文件");
    stream_fail.store(false, Ordering::Relaxed);
    std::fs::remove_file(tmp_mp3).unwrap();

    // 自添封面可移除。
    app.hndlcmd(AppCommand::RemoveDeck(0));
    assert!(app.ssht().error.is_some());
    app.hndlcmd(AppCommand::RemoveDeck(2));
    assert!(app.ssht().error.is_none());
    assert!(storage.lddecks().unwrap().is_empty());
}
