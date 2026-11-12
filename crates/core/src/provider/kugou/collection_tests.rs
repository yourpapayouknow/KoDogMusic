// 歌单与专辑联网回归。
use super::{client::KugouClient, library::rowtrk};
use crate::deps::{serde_json::{json, Value}, md5::{Digest, Md5}};

/// 读取已核实的三组公开曲目。
fn fixtures() -> Value { crate::deps::serde_json::from_str(include_str!("../../../tests/fixtures/collections.json")).unwrap() }

/// 读取完整公开集合；可将已核实的元数据保存为离线回归样本。
#[test]
#[ignore = "需要官方公开专辑和歌单接口，不修改账号"]
fn tst_collnet() {
    let client = KugouClient::nw();
    let mut cohorts = Vec::new();
    for (id, name, kind, expected) in [("966846", "叶惠美", "录音室专辑", 11), ("97939899", "暮色回响", "单曲专辑", 1)] {
        let root = client.req("https://gateway.kugou.com/v1/album_audio/lite", client.params(), Some(&json!({"album_id":id,"is_buy":"","page":1,"pagesize":30})), false, "openapi.kugou.com").unwrap();
        let data = root.get("data").unwrap();
        let rows = data.get("songs").and_then(Value::as_array).unwrap();
        assert_eq!(rows.len(), expected);
        assert_eq!(data.get("total").and_then(Value::as_u64), Some(expected as u64));
        let detail = client.req("https://gateway.kugou.com/kmr/v2/albums", client.params(), Some(&json!({"data":[{"album_id":id}],"is_buy":0,"fields":"album_id,album_name,type"})), false, "openapi.kugou.com").unwrap();
        let album = &detail["data"][0];
        assert_eq!(album["album_name"].as_str(), Some(name));
        assert_eq!(album["type"].as_str(), Some(kind));
        let tracks: Vec<_> = rows.iter().map(|row| rowtrk(&json!({"hash":row["audio_info"]["hash"],"name":row["base"]["audio_name"],
            "author_name":row["base"]["author_name"],"timelen":row["audio_info"]["duration"],"cover":row["album_info"]["cover"]})).unwrap().0).collect();
        assert!(tracks.iter().all(|track| !track.title.is_empty() && track.duration_ms.is_some_and(|value| value > 0)));
        eprintln!("verified {kind} · {name} · {} tracks", tracks.len());
        cohorts.push(json!({"kind":kind,"name":name,"public_id":id,"tracks":tracks}));
    }
    let params = client.params();
    let time = params.get("clienttime").unwrap();
    let body = json!({"appid":3116,"mid":client.sess().mid,"clientver":11440,"platform":"android","clienttime":time,"userid":0,"module_id":1,"page":1,"pagesize":5,
        "key":format!("{:x}", Md5::digest(format!("3116LnT6xpN3khm36zse0QzvmgTZ3waWdRSA11440{time}"))),
        "special_recommend":{"withtag":1,"withsong":1,"sort":1,"ugc":1,"is_selected":0,"withrecommend":1,"area_code":1,"categoryid":0},"req_multi":1,"retrun_min":5,"return_special_falg":1});
    let root = client.req("https://gateway.kugou.com/v2/special_recommend", params, Some(&body), false, "specialrec.service.kugou.com").unwrap();
    let first = root["data"]["special_list"].as_array().unwrap().first().unwrap();
    let id = first["global_collection_id"].as_str().unwrap();
    let name = first["specialname"].as_str().unwrap();
    let mut tracks = Vec::new();
    loop {
        let mut params = client.params();
        for (key, value) in [("area_code","1"),("plat","1"),("type","1"),("mode","1"),("personal_switch","1"),("extend_fields","abtags,hot_cmt,popularization"),("pagesize","100"),("global_collection_id",id)] { params.insert(key.into(), value.into()); }
        params.insert("begin_idx".into(), tracks.len().to_string());
        let root = client.req("https://gateway.kugou.com/pubsongs/v2/get_other_list_file_nofilt", params, None, false, "").unwrap();
        let data = &root["data"];
        let total = data["count"].as_u64().unwrap() as usize;
        assert!(total > 0 && total <= 5000);
        let rows = data["songs"].as_array().unwrap();
        assert!(!rows.is_empty());
        tracks.extend(rows.iter().map(|row| rowtrk(row).unwrap().0));
        if tracks.len() >= total { assert_eq!(tracks.len(), total); break; }
    }
    eprintln!("verified playlist · {name} · {} tracks", tracks.len());
    cohorts.push(json!({"kind":"歌单","name":name,"public_id":id,"tracks":tracks}));
    if let Some(path) = std::env::var_os("KODOG_COHORT_FILE") {
        let path = std::path::PathBuf::from(path);
        assert!(path.is_absolute() && !path.exists(), "样本输出必须是新的绝对路径");
        std::fs::write(path, crate::deps::serde_json::to_vec_pretty(&cohorts).unwrap()).unwrap();
    }
}

/// 用隔离队列逐首启动真实设备输出，并下载校验一张Single的高品质及无损音源。
#[test]
#[ignore = "需要当前已登录账号、网络和Windows音频设备，不修改云端收藏"]
fn tst_collaudio() {
    use crate::{application::{coordinator::Coordinator, commands::AppCommand, ports::{PlaybackPort, StoragePort}},
        audio::{controller::AudioController, decoder::Decoder}, domain::{track::{TrackSummary, AudioQuality}, playback::{RepeatMode, PlaybackState}},
        provider::mock::MockProvider, storage::database::DatabaseStorage, services::downloads::{DownloadTask, DownloadState}};
    use std::{sync::Arc, time::{Instant, Duration}};
    let root = std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap()).join("KoDogLite/secrets.dat");
    let client = KugouClient::opn(root).unwrap();
    assert_ne!(client.sess().user_id, 0);
    let cohorts = fixtures();
    let provider = Arc::new(MockProvider::nw());
    let stream = Arc::new(super::stream::KugouStream::nw(client));
    for cohort in cohorts.as_array().unwrap() {
        let tracks: Vec<TrackSummary> = crate::deps::serde_json::from_value(cohort["tracks"].clone()).unwrap();
        let storage = Arc::new(DatabaseStorage::inmem().unwrap()); storage.svtrks(&tracks).unwrap();
        let player = Arc::new(AudioController::nw());
        let mut app = Coordinator::nw(provider.clone(), provider.clone(), provider.clone(), stream.clone(), player.clone(), storage);
        app.hndlcmd(AppCommand::SetVolume(0.1)); app.hndlcmd(AppCommand::SetRepeat(RepeatMode::Off));
        for track in &tracks { app.hndlcmd(AppCommand::QueueAdd(track.id.clone())); }
        app.hndlcmd(AppCommand::QueuePlay(0));
        for (index, track) in tracks.iter().enumerate() {
            assert!(app.ssht().error.is_none(), "{}: {:?}", track.title, app.ssht().error);
            assert_eq!(app.ssht().queue_cursor, Some(index));
            let deadline = Instant::now() + Duration::from_secs(20);
            loop {
                app.tick();
                assert_ne!(app.ssht().playback, PlaybackState::Failed, "{}: 音频启动失败", track.title);
                if app.ssht().position_ms >= 500 { break; }
                assert!(Instant::now() < deadline, "{}: 音频消费未推进", track.title);
                std::thread::sleep(Duration::from_millis(50));
            }
            eprintln!("device output · {} · {}/{} · {}", cohort["name"], index + 1, tracks.len(), track.title);
            if index + 1 < tracks.len() { app.hndlcmd(AppCommand::Next); }
        }
        let deadline = Instant::now() + Duration::from_secs(15);
        app.hndlcmd(AppCommand::Seek(app.ssht().duration_ms.saturating_sub(300)));
        while app.ssht().playback != PlaybackState::Idle {
            app.tick(); assert!(app.ssht().error.is_none(), "{:?}", app.ssht().error);
            assert!(Instant::now() < deadline, "末曲未停止: {}", cohort["name"]);
            std::thread::sleep(Duration::from_millis(20));
        }
        if tracks.len() > 1 {
            app.hndlcmd(AppCommand::QueuePlay(0));
            let deadline = Instant::now() + Duration::from_secs(15);
            while app.ssht().duration_ms == 0 { app.tick(); assert!(Instant::now() < deadline); std::thread::sleep(Duration::from_millis(20)); }
            app.hndlcmd(AppCommand::Seek(app.ssht().duration_ms.saturating_sub(300)));
            while app.ssht().queue_cursor == Some(0) {
                app.tick(); assert!(app.ssht().error.is_none(), "{:?}", app.ssht().error);
                assert!(Instant::now() < deadline, "首曲结束后未续播: {}", cohort["name"]);
                std::thread::sleep(Duration::from_millis(20));
            }
            assert_eq!(app.ssht().queue_cursor, Some(1));
        }
        eprintln!("verified real EOS · {} · last stops, next follows order", cohort["name"]);
        player.stp();
        if tracks.len() == 1 {
            let directory = std::env::temp_dir().join(format!("kodog-live-download-{}", crate::deps::uuid::Uuid::new_v4()));
            for quality in [AudioQuality::High, AudioQuality::Lossless] {
                let task = DownloadTask::nw(tracks[0].clone(), quality, directory.clone(), stream.clone()).unwrap();
                let deadline = Instant::now() + Duration::from_secs(90);
                while !task.done() { assert!(Instant::now() < deadline); std::thread::sleep(Duration::from_millis(100)); }
                let job = task.state.lock().unwrap().clone();
                assert_eq!(job.state, DownloadState::Completed);
                let mut decoder = Decoder::opnfil(&job.dest_path).unwrap();
                assert!(decoder.nxtchk(&mut Vec::new()).unwrap());
                eprintln!("verified download · {:?} · {} bytes", quality, std::fs::metadata(&job.dest_path).unwrap().len());
            }
            std::fs::remove_dir_all(directory).unwrap();
        }
    }
}

/// 构造独立的桌面验收档案，不替换用户队列或配置。
#[test]
#[ignore = "需指定全新绝对档案路径，仅用于Release完整集合验收"]
fn tst_profile() {
    use crate::{storage::database::DatabaseStorage, application::ports::StoragePort, domain::track::TrackSummary};
    let root = std::path::PathBuf::from(std::env::var_os("KODOG_TEST_PROFILE").expect("必须指定独立测试档案路径"));
    assert!(root.is_absolute() && !root.exists());
    let directory = root.join("KoDogLite");
    std::fs::create_dir_all(&directory).unwrap();
    let current = std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap()).join("KoDogLite");
    std::fs::copy(current.join("secrets.dat"), directory.join("secrets.dat")).unwrap();
    let storage = DatabaseStorage::opn(directory.join("library.db"), directory.join("config.toml")).unwrap();
    let tracks: Vec<TrackSummary> = fixtures().as_array().unwrap().iter().flat_map(|cohort|
        crate::deps::serde_json::from_value::<Vec<TrackSummary>>(cohort["tracks"].clone()).unwrap()).collect();
    storage.svtrks(&tracks).unwrap();
    storage.svqu(&tracks.iter().map(|track| track.id.clone()).collect::<Vec<_>>(), Some(0)).unwrap();
    storage.svcfg(&crate::domain::settings::AppSettings::default()).unwrap();
    eprintln!("prepared isolated profile: {} public tracks; encrypted session reused", tracks.len());
}

/// 只读验证新接入的设备注册、蔡琴搜索与专辑、每日推荐、推荐歌单、权限品质与用户歌单列表接口。
#[test]
#[ignore = "需要联网及已扫码账号的只读副本，不修改任何云端歌单"]
fn tst_live_ro() {
    use crate::application::ports::{CatalogPort, LibraryPort};
    use crate::provider::kugou::{catalog::KugouCatalog, library::KugouLibrary};
    let fresh = KugouClient::nw();
    fresh.regdev().expect("匿名设备注册 dfid 应成功");
    assert!(fresh.sess().dfid_registered);

    let orig = std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap()).join("KoDogLite/secrets.dat");
    let tmp = std::env::temp_dir().join(format!("kodog-ro-{}.dat", crate::deps::uuid::Uuid::new_v4()));
    std::fs::copy(&orig, &tmp).unwrap();
    let client = KugouClient::opn(&tmp).unwrap();
    let _ = std::fs::remove_file(&tmp);

    let cat = KugouCatalog::nw(client.clone());
    let lib = KugouLibrary::nw(client.clone());

    let res = cat.srchtrks("蔡琴 迟来的人", 1, 5).unwrap();
    assert!(!res.is_empty(), "搜索迟来的人应返回结果");
    for (i, t) in res.iter().enumerate() {
        eprintln!("search[{i}]: title={} artist={} album_id={:?} album_name={:?}", t.title, t.artist_text, t.album_id, t.album_name);
    }
    let first = res.iter().find(|t| t.album_id.is_some()).expect("搜索结果应含 album_id");
    let aid = first.album_id.as_deref().unwrap();
    let dtl = client.albdtl(aid);
    let sngs = client.albsngs(aid, 1, 30);
    eprintln!("albdtl({aid})={:?}\nalbsngs({aid})={:?}", dtl.as_ref().map(|v| &v["status"]), sngs.as_ref().map(|v| &v["status"]));
    let (alb_name, _, alb_tracks) = cat.albdtl(aid).unwrap();
    assert!(!alb_tracks.is_empty(), "专辑曲目不应为空");
    let quals = cat.prvqlt(&alb_tracks).unwrap();
    assert!(!quals.is_empty(), "权限品质检测不应为空");
    eprintln!("search+album+privilege OK: {} ({}首), 品质={:?}", alb_name, alb_tracks.len(), quals);

    let rec = cat.recpl(8).unwrap();
    assert!(!rec.is_empty(), "推荐歌单不应为空");
    let first_gid = rec[0].global_id.as_deref().unwrap();
    let rec_tracks = cat.pubpltrks(first_gid).unwrap();
    assert!(!rec_tracks.is_empty(), "推荐歌单曲目不应为空");
    eprintln!("recpl={} first_playlist={} ({}首)", rec.len(), rec[0].name, rec_tracks.len());
    if client.sess().user_id != 0 {
        let pls = lib.usrpls().unwrap();
        assert!(pls.iter().any(|p| p.is_like), "用户歌单应识别出 Like");
        let daily = cat.dailyrec().unwrap();
        assert!(!daily.is_empty(), "每日推荐不应为空");
        eprintln!("usrpls={} dailyrec={}", pls.len(), daily.len());
    } else {
        eprintln!("current secrets.dat is Guest; skipping auth-only usrpls/dailyrec checks");
    }
}
