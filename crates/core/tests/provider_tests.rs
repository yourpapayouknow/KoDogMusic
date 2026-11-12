use kugou_core::deps::serde_json;
use kugou_core::provider::kugou::dto::SearchSongResp;
use kugou_core::provider::kugou::client::KugouClient;
use std::collections::BTreeMap;

/// 锁定原生协议按完整参数串排序及UTF8签名结果。
#[test]
fn tst_sigmd5() {
    let params = BTreeMap::from([
        ("a".to_string(), "空 格".to_string()),
        ("a2".to_string(), "2".to_string()),
        ("z".to_string(), "&".to_string()),
    ]);
    assert_eq!(KugouClient::sigmd5(&params), "4bdd03fc792d4a425687dd86d2213d5c");
}

/// 验证公开搜索响应及缺失必要数据的处理。
#[test]
fn tst_srchdto() {
    let raw = r#"{"status":1,"errcode":0,"data":{"total":1,"info":[{"hash":"abc","songname":"歌曲","singername":"歌手","duration":200,"album_id":42}]}}"#;
    let response: SearchSongResp = serde_json::from_str(raw).unwrap();
    let songs = response.data.unwrap().lists;
    assert_eq!(response.err_code, 0);
    assert_eq!(songs.len(), 1);
    assert_eq!(songs[0].file_hash, "abc");
    assert_eq!(songs[0].song_name, "歌曲");
    assert_eq!(songs[0].duration, 200);
    let empty: SearchSongResp = serde_json::from_str(r#"{"status":1,"errcode":0,"data":{"total":0,"info":[]}}"#).unwrap();
    assert!(empty.data.unwrap().lists.is_empty());
    assert!(serde_json::from_str::<SearchSongResp>(r#"{"status":1,"data":{"total":1,"info":[{}]}}"#).is_err());
}
