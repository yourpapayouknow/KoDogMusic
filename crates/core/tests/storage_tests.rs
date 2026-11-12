use kugou_core::application::ports::StoragePort;
use kugou_core::domain::settings::ThemeMode;
use kugou_core::domain::ids::TrackId;
use kugou_core::domain::track::TrackSummary;
use kugou_core::deps::rusqlite::Connection;
use kugou_core::storage::database::DatabaseStorage;

/// 测试存储模块初始化与配置读写
#[test]
fn tst_stginit() {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let stem = format!("kg-stg-{}-{stamp}", std::process::id());
    let tmp_db = std::env::temp_dir().join(format!("{stem}.db"));
    let tmp_cfg = std::env::temp_dir().join(format!("{stem}.toml"));

    let stg = DatabaseStorage::opn(&tmp_db, &tmp_cfg).expect("Storage init failed");
    let mut cfg = stg.ldcfg().expect("Load config failed");

    assert_eq!(cfg.appearance.theme, ThemeMode::System);
    assert!((cfg.audio.volume - 0.5).abs() < f32::EPSILON);

    cfg.audio.volume = 0.85;
    cfg.appearance.theme = ThemeMode::Dark;
    stg.svcfg(&cfg).expect("Save config failed");

    let reloaded = stg.ldcfg().expect("Reload config failed");
    assert!((reloaded.audio.volume - 0.85).abs() < f32::EPSILON);
    assert_eq!(reloaded.appearance.theme, ThemeMode::Dark);

    let mut track = TrackSummary::nw(TrackId::kg("saved"), "保存的歌曲", "歌手");
    track.cover_key = Some("cover".into());
    track.duration_ms = Some(120000);
    track.album_id = Some("966846".into());
    track.album_name = Some("叶惠美".into());
    track.album_audio_id = Some(42);
    track.extra_hashes.insert("320".into(), "h320".into());
    stg.svtrks(std::slice::from_ref(&track)).unwrap();
    stg.svqu(std::slice::from_ref(&track.id), Some(0)).unwrap();
    stg.stfvrt(&track.id, true).unwrap();
    let deck = kugou_core::domain::track::CoverDeck {
        id: "album:966846".into(),
        kind: kugou_core::domain::track::DeckKind::Album,
        title: "叶惠美".into(),
        cover_key: Some("cover".into()),
        source_id: "966846".into(),
        removable: true,
        tracks_loaded: true,
        tracks: vec![track.clone()],
        cursor: Some(0),
    };
    stg.svdecks(std::slice::from_ref(&deck)).unwrap();
    let audio_file = tmp_db.with_extension("flac");
    std::fs::write(&audio_file, b"fLaC").unwrap();
    let dl = kugou_core::domain::track::DownloadedFile {
        track_id: track.id.clone(),
        quality: kugou_core::domain::track::AudioQuality::Lossless,
        is_lossless: true,
        collection_name: "叶惠美".into(),
        file_path: audio_file.display().to_string(),
    };
    stg.svdwnld(&dl).unwrap();
    let reopened = DatabaseStorage::opn(&tmp_db, &tmp_cfg).unwrap();
    assert_eq!(reopened.ldtrk(&track.id).unwrap(), Some(track.clone()));
    assert_eq!(reopened.ldfvrt().unwrap(), vec![track.clone()]);
    assert_eq!(reopened.ldqu().unwrap(), (vec![track.id.clone()], Some(0)));
    assert_eq!(reopened.lddecks().unwrap(), vec![deck]);
    assert_eq!(reopened.lddwnld(&track.id).unwrap(), Some(dl));
    drop(reopened);
    std::fs::remove_file(audio_file).unwrap();

    // 失败保留队列与游标。
    let probe = Connection::open(&tmp_db).unwrap();
    probe.execute("INSERT INTO tracks (provider, track_id, title, artist_text, updated_at) VALUES ('kugou', 'dirty_mp3', '测试曲目.mp3', '歌手', 0)", []).unwrap();
    assert_eq!(stg.ldtrk(&TrackId::kg("dirty_mp3")).unwrap().unwrap().title, "测试曲目");
    probe.execute_batch("CREATE TRIGGER reject_track BEFORE INSERT ON queue_items WHEN NEW.track_id='reject' BEGIN SELECT RAISE(ABORT, 'test rejection'); END;").unwrap();
    assert!(stg.svqu(&[track.id.clone(), TrackId::kg("reject")], Some(1)).is_err());
    assert_eq!(stg.ldqu().unwrap(), (vec![track.id.clone()], Some(0)));
    assert!(stg.svqu(&[], Some(0)).is_err());
    stg.stfvrt(&track.id, false).unwrap();
    assert!(stg.ldfvrt().unwrap().is_empty());
    drop(probe);

    // 损坏配置必须显式报错。
    std::fs::write(&tmp_cfg, "[audio\nvolume = 0.5").unwrap();
    assert!(stg.ldcfg().is_err());
    drop(stg);
    std::fs::remove_file(tmp_db).unwrap();
    std::fs::remove_file(tmp_cfg).unwrap();
}
