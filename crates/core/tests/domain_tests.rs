use kugou_core::audio::ring::RealtimeRing;
use kugou_core::domain::ids::TrackId;
use kugou_core::domain::playback::{PlaybackState, RepeatMode};
use kugou_core::domain::queue::Queue;

/// 测试播放状态机枚举定义与默认值
#[test]
fn tst_stmach() {
    let state = PlaybackState::default();
    assert_eq!(state, PlaybackState::Idle);

    let states = vec![
        PlaybackState::Idle,
        PlaybackState::Resolving,
        PlaybackState::Buffering,
        PlaybackState::Playing,
        PlaybackState::Paused,
        PlaybackState::Seeking,
        PlaybackState::Recovering,
        PlaybackState::Ended,
        PlaybackState::Failed,
    ];
    assert_eq!(states.len(), 9);
}

/// 测试循环模式枚举定义与默认值
#[test]
fn tst_loopmd() {
    let mode = RepeatMode::default();
    assert_eq!(mode, RepeatMode::Off);
    assert_eq!(kugou_core::domain::track::AudioQuality::default(), kugou_core::domain::track::AudioQuality::Lossless);
    assert_eq!(kugou_core::domain::track::clnttl("你不要那样看着我的眼睛.mp3"), "你不要那样看着我的眼睛");
    assert_eq!(kugou_core::domain::track::clnttl("Overcast Sky.FLAC"), "Overcast Sky");
    assert_eq!(kugou_core::domain::track::clnttl(".mp3"), ".mp3");

    assert_ne!(RepeatMode::Off, RepeatMode::One);
    assert_ne!(RepeatMode::All, RepeatMode::One);
}

/// 测试队列增删与前后切歌逻辑
#[test]
fn tst_quenc() {
    let mut q = Queue::nw();
    assert!(q.crr().is_none());

    let id1 = TrackId::kg("t1");
    let id2 = TrackId::kg("t2");

    q.psh(id1.clone());
    q.psh(id2.clone());
    assert_eq!(q.items.len(), 2);

    let nxt = q.nxt();
    assert_eq!(nxt, Some(&id1));

    let nxt2 = q.nxt();
    assert_eq!(nxt2, Some(&id2));

    let prv = q.prv();
    assert_eq!(prv, Some(&id1));
    q.shuffle = true;
    for _ in 0..32 {
        let previous = q.crr().cloned();
        assert_ne!(q.nxt().cloned(), previous);
    }
    q.shuffle = false;
    q.repeat_mode = RepeatMode::Off;
    q.cursor = None;
    assert_eq!(q.nxt(), Some(&id1));
    assert_eq!(q.nxt(), Some(&id2));
    assert_eq!(q.nxt(), None);
}

/// 测试SPSC无锁环形缓冲区读写
#[test]
fn tst_rngbuf() {
    let ring = RealtimeRing::nw(1024);
    assert_eq!(ring.buflen(), 0);
    assert!(ring.isempty());

    let data = vec![1.0f32, 2.0, 3.0, 4.0];
    let (written, frames) = ring.psh(&data, 2, 1024);
    assert_eq!(written, 4);
    assert_eq!(frames, 2);
    assert_eq!(ring.buflen(), 4);

    let mut out = vec![0.0f32; 4];
    let (read, read_frame) = ring.pop(&mut out);
    assert_eq!(read, 4);
    assert_eq!(read_frame, 2);
    assert_eq!(out, data);
    assert_eq!(ring.buflen(), 0);

    // 写入上限跟随容量。
    let big = RealtimeRing::nw(200_000);
    assert_eq!(big.cap(), 200_000);
    let chunk = vec![0.5f32; 100_000];
    assert_eq!(big.psh(&chunk, 50_000, big.cap()).0, 100_000);
    assert_eq!(big.psh(&chunk, 50_000, big.cap()).0, 100_000);
    assert_eq!(big.psh(&chunk, 50_000, big.cap()).0, 0);
    assert_eq!(big.buflen(), 200_000);
}
