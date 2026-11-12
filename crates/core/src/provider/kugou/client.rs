use std::collections::BTreeMap;
use std::io::Read;
use crate::deps::reqwest::blocking::{Client, Response};
use crate::deps::serde_json::Value;
use crate::domain::error::{AppError, NetworkError};
use crate::domain::track::AudioQuality;
use crate::provider::kugou::dto::SearchSongResp;

const SALT_LITE: &str = "LnT6xpN3khm36zse0QzvmgTZ3waWdRSA";
const USER_AGENT: &str = "Android15-1070-11083-46-0-DiscoveryDRADProtocol-wifi";
const JSON_LIMIT: u64 = 2 * 1024 * 1024;
const KEY_SALT: &str = "185672dd44712f60bb1736df5a377e82";
const SALT_WEB: &str = "NVPh5oo715z5DIWAeQlhMDsWXXQV4hwt";
const LITE_MOD: &str = "c40a2d0da76511f3bb1cc2bbd3afbd8bea83b4d6b05b6c13eb8920c53f1af7679b32ba0d0edb843240ef1b836efed3ee240734c14c1399fd6594d16af22f52525d14d72e0155c6dcc8638d4f7bb94f3a0b1f4c29f991972f2a160a25eb0a9e724336be7f69bbd319ffab1c6dd8470b021dc434f3faba89f4a2a01b33bdbdd08b";

/// 保存原生协议设备标识与登录凭据。
#[derive(Clone, crate::deps::serde::Serialize, crate::deps::serde::Deserialize)]
pub(crate) struct Session {
    pub mid: String,
    pub dfid: String,
    #[serde(default)]
    pub guid: String,
    #[serde(default)]
    pub dfid_registered: bool,
    pub user_id: u64,
    pub token: String,
    #[serde(default)]
    pub nickname: String,
    #[serde(default)]
    pub avatar: Option<String>,
}

/// 酷狗网络请求客户端
#[derive(Clone)]
pub struct KugouClient {
    http: Client,
    session: std::sync::Arc<std::sync::Mutex<Session>>,
    secrets: Option<std::path::PathBuf>,
}

impl KugouClient {
    /// 构造酷狗客户端
    pub fn nw() -> Self {
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .unwrap_or_default();
        // 沿用参考MID规则。
        let guid = Self::newguid();
        let mid = u128::from_str_radix(&Self::mdhex(guid.as_bytes()), 16).unwrap().to_string();
        let dfid = crate::deps::uuid::Uuid::new_v4().simple().to_string()[..24].to_uppercase();
        Self {
            http,
            session: std::sync::Arc::new(std::sync::Mutex::new(Session {
                mid, dfid, guid, dfid_registered: false,
                user_id: 0, token: String::new(), nickname: String::new(), avatar: None,
            })),
            secrets: None,
        }
    }

    /// 读取或创建当前Windows用户加密的设备与会话。
    pub fn opn(path: impl Into<std::path::PathBuf>) -> Result<Self, AppError> {
        let mut client = Self::nw();
        client.secrets = Some(path.into());
        let path = client.secrets.as_ref().unwrap();
        match std::fs::File::open(path) {
            Ok(file) => {
                let mut bytes = Vec::new();
                file.take(128 * 1024 + 1).read_to_end(&mut bytes).map_err(|_| crate::domain::error::StorageError::Permission)?;
                let plain = crate::deps::Zeroizing::new(crate::storage::secrets::prtct(&bytes, true)?);
                let mut session: Session = crate::deps::serde_json::from_slice(&plain).map_err(|_| crate::domain::error::StorageError::Corrupt)?;
                if session.mid.parse::<u128>().is_err() || session.dfid.len() != 24 || session.token.len() > 1024 {
                    return Err(crate::domain::error::StorageError::Corrupt.into());
                }
                if session.guid.is_empty() {
                    session.guid = Self::newguid();
                }
                *client.session.lock().unwrap() = session;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => client.svsess(client.sess())?,
            Err(_) => return Err(crate::domain::error::StorageError::Permission.into()),
        }
        Ok(client)
    }

    /// 读取同一客户端各适配器共享的会话。
    pub(crate) fn sess(&self) -> Session { self.session.lock().unwrap().clone() }

    /// 原子保存DPAPI密文后发布共享会话。
    pub(crate) fn svsess(&self, session: Session) -> Result<(), AppError> {
        if let Some(path) = &self.secrets {
            if let Some(parent) = path.parent() { std::fs::create_dir_all(parent).map_err(|_| crate::domain::error::StorageError::Permission)?; }
            let plain = crate::deps::Zeroizing::new(crate::deps::serde_json::to_vec(&session).map_err(|_| crate::domain::error::StorageError::Corrupt)?);
            let encrypted = crate::storage::secrets::prtct(&plain, false)?;
            crate::storage::database::wrtatom(path, &encrypted)?;
        }
        *self.session.lock().unwrap() = session;
        Ok(())
    }

    /// 按需向酷狗注册设备指纹并持久化 dfid。
    pub fn regdev(&self) -> Result<(), AppError> {
        use crate::deps::rand::Rng;
        let session = self.sess();
        if session.dfid_registered {
            return Ok(());
        }
        let guid = if session.guid.is_empty() { Self::newguid() } else { session.guid.clone() };
        let devinfo = crate::deps::serde_json::json!({
            "availableRamSize": 4983533568u64, "availableRomSize": 48114719u64, "availableSDSize": 48114717u64,
            "basebandVer": "", "batteryLevel": 100, "batteryStatus": 3, "brand": "Redmi", "buildSerial": "unknown",
            "device": "marble", "imei": guid, "imsi": "", "manufacturer": "Xiaomi", "uuid": guid,
            "accelerometer": false, "accelerometerValue": "", "gravity": false, "gravityValue": "",
            "gyroscope": false, "gyroscopeValue": "", "light": false, "lightValue": "",
            "magnetic": false, "magneticValue": "", "orientation": false, "orientationValue": "",
            "pressure": false, "pressureValue": "", "step_counter": false, "step_counterValue": "",
            "temperature": false, "temperatureValue": "",
        });
        let plain = crate::deps::serde_json::to_string(&devinfo).unwrap();
        let mut rng = crate::deps::rand::thread_rng();
        let aes_key = Self::aeskey(&mut rng);
        let body = aesenc(&aes_key, plain.as_bytes())?;
        let uid = if session.user_id != 0 { Value::String(session.user_id.to_string()) } else { Value::from(0) };
        let rsa_in = crate::deps::serde_json::to_string(&crate::deps::serde_json::json!({
            "aes": aes_key, "uid": uid, "token": session.token,
        })).unwrap();
        let mut fill = [0u8; 64];
        rng.fill(&mut fill);
        let p = pkcs1blk(rsa_in.as_bytes(), &fill)?;
        let mut params = self.params();
        params.insert("part".into(), "1".into());
        params.insert("platid".into(), "1".into());
        params.insert("p".into(), p);
        params.insert("signature".into(), Self::sgn(&params, &body, false));
        let mut request = self.http.post("https://userservice.kugou.com/risk/v2/r_register_dev")
            .query(&params).header("User-Agent", USER_AGENT)
            .header("mid", &session.mid).header("dfid", &session.dfid)
            .header("kg-rc", "1").header("kg-rec", "1").header("kg-thash", "5d816a0")
            .header("kg-rf", "B9EDA08A64250DEFFBCADDEE00F8F25F").body(body);
        if let Some(time) = params.get("clienttime") { request = request.header("clienttime", time); }
        let response = request.send().map_err(neterr)?.error_for_status().map_err(neterr)?;
        let mut raw = Vec::new();
        response.take(JSON_LIMIT + 1).read_to_end(&mut raw).map_err(|_| NetworkError::Server("读取设备注册响应失败".into()))?;
        let dec = aesdec(&aes_key, &raw)?;
        let val: Value = crate::deps::serde_json::from_slice(&dec).map_err(|_| NetworkError::Server("设备注册响应解析失败".into()))?;
        let dfid = val.get("data").and_then(|d| d.get("dfid").or_else(|| d.get("DFID"))).or_else(|| val.get("dfid"))
            .and_then(Value::as_str).filter(|s| s.len() >= 16)
            .ok_or_else(|| NetworkError::Server("设备注册未返回有效dfid".into()))?;
        let mut next = self.sess();
        next.dfid = dfid.to_string();
        next.guid = guid;
        next.dfid_registered = true;
        self.svsess(next)
    }

    /// 对参数字典进行排序并加盐计算 MD5 签名
    pub fn sigmd5(params: &BTreeMap<String, String>) -> String {
        Self::sgn(params, "", false)
    }

    /// 签署原生或Web协议的参数与精确请求体。
    pub(crate) fn sgn(params: &BTreeMap<String, String>, body: &str, web: bool) -> String {
        let mut pairs: Vec<String> = params
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        pairs.sort();
        let salt = if web { SALT_WEB } else { SALT_LITE };
        let payload = format!("{salt}{}{body}{salt}", pairs.join(""));
        Self::mdhex(payload.as_bytes())
    }

    /// 构造参考原生客户端的默认设备参数。
    pub(crate) fn params(&self) -> BTreeMap<String, String> {
        let session = self.sess();
        let mut params = BTreeMap::from([
            ("mid".into(), session.mid), ("dfid".into(), session.dfid), ("uuid".into(), "-".into()),
            ("appid".into(), "3116".into()), ("clientver".into(), "11440".into()),
            ("clienttime".into(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs().to_string()),
        ]);
        if session.user_id != 0 && !session.token.is_empty() {
            params.insert("userid".into(), session.user_id.to_string());
            params.insert("token".into(), session.token);
        }
        params
    }

    /// 发送有界签名请求并保留服务端真实错误码。
    pub(crate) fn req(&self, url: &str, params: BTreeMap<String, String>, body: Option<&Value>, web: bool, router: &str) -> Result<Value, AppError> {
        self.reqraw(url, params, body.map(|value| crate::deps::serde_json::to_string(value).unwrap()), web, router, "application/json")
    }

    /// 按实际HTTP请求体签名，支持官方空表单POST。
    pub(crate) fn reqraw(&self, url: &str, mut params: BTreeMap<String, String>, body: Option<String>, web: bool, router: &str, content_type: &str) -> Result<Value, AppError> {
        params.insert("signature".into(), Self::sgn(&params, if web { "" } else { body.as_deref().unwrap_or("") }, web));
        let session = self.sess();
        let mut request = if let Some(body) = body { self.http.post(url).header("Content-Type", content_type).body(body) } else { self.http.get(url) };
        request = request.query(&params).header("User-Agent", USER_AGENT).header("mid", session.mid).header("dfid", session.dfid)
            .header("kg-rc", "1").header("kg-rec", "1").header("kg-thash", "5d816a0").header("kg-rf", "B9EDA08A64250DEFFBCADDEE00F8F25F");
        if let Some(time) = params.get("clienttime") { request = request.header("clienttime", time); }
        if !router.is_empty() { request = request.header("x-router", router); }
        if router == "openapi.kugou.com" { request = request.header("kg-tid", "255"); }
        let root = rdjson(request.send().map_err(neterr)?)?;
        let code = root.get("error_code").or_else(|| root.get("err_code")).or_else(|| root.get("errcode")).and_then(Value::as_i64).unwrap_or(0);
        let lyric_ok = url.starts_with("https://lyrics.kugou.com/") && code == 200;
        if root.get("status").and_then(Value::as_i64) == Some(0) || (code != 0 && !lyric_ok) {
            return Err(NetworkError::Server(format!("接口返回失败，错误码{code}")).into());
        }
        Ok(root)
    }

    /// 获取真实LRC文本，未收录歌词时返回空文本。
    pub(crate) fn lyrtxt(&self, hash: &str) -> Result<String, AppError> {
        use crate::deps::base64::Engine;
        let mut params = BTreeMap::new();
        for (key, value) in [("album_audio_id", "0"), ("appid", "3116"), ("clientver", "11440"), ("duration", "0"),
            ("hash", hash), ("keyword", ""), ("lrctxt", "1"), ("man", "no")] { params.insert(key.into(), value.into()); }
        let root = self.req("https://lyrics.kugou.com/v1/search", params, None, false, "")?;
        let data = root.get("data").unwrap_or(&root);
        let Some(candidate) = data.get("candidates").and_then(Value::as_array).and_then(|items| items.first()) else { return Ok(String::new()); };
        let id = candidate.get("id").and_then(|value| value.as_str().map(ToString::to_string).or_else(|| value.as_u64().map(|value| value.to_string())))
            .ok_or_else(|| NetworkError::Server("歌词缺少标识".into()))?;
        let key = candidate.get("accesskey").and_then(Value::as_str).ok_or_else(|| NetworkError::Server("歌词缺少访问凭据".into()))?;
        let mut params = self.params();
        for (name, value) in [("ver", "1"), ("client", "android"), ("id", id.as_str()), ("accesskey", key), ("fmt", "lrc"), ("charset", "utf8")] { params.insert(name.into(), value.into()); }
        let root = self.req("https://lyrics.kugou.com/download", params, None, false, "")?;
        let content = root.get("content").and_then(Value::as_str).ok_or_else(|| NetworkError::Server("歌词内容缺失".into()))?;
        let bytes = crate::deps::base64::engine::general_purpose::STANDARD.decode(content).map_err(|_| NetworkError::Server("歌词编码无效".into()))?;
        String::from_utf8(bytes).map_err(|_| NetworkError::Server("歌词字符编码无效".into()).into())
    }

    /// 有界读取远程图片，不附带账户参数。
    pub fn blob(&self, url: &str, maximum: usize) -> Result<Vec<u8>, AppError> {
        let parsed = crate::deps::url::Url::parse(url).map_err(|_| NetworkError::Server("图片地址无效".into()))?;
        if !matches!(parsed.scheme(), "http" | "https") { return Err(NetworkError::Server("图片地址协议无效".into()).into()); }
        let response = self.http.get(parsed).timeout(std::time::Duration::from_secs(8)).send().map_err(neterr)?.error_for_status().map_err(neterr)?;
        if response.content_length().is_some_and(|size| size > maximum as u64) { return Err(NetworkError::Server("图片超过大小限制".into()).into()); }
        let mut bytes = Vec::new();
        response.take(maximum as u64 + 1).read_to_end(&mut bytes).map_err(|_| NetworkError::Server("图片读取失败".into()))?;
        if bytes.len() > maximum { return Err(NetworkError::Server("图片超过大小限制".into()).into()); }
        Ok(bytes)
    }

    /// 按参考协议加密并请求真实用户资料。
    pub(crate) fn usrdtl(&self) -> Result<Value, AppError> {
        let session = self.sess();
        if session.user_id == 0 || session.token.is_empty() { return Err(crate::domain::error::AuthError::Missing.into()); }
        let mut params = self.params();
        let time = params["clienttime"].parse::<u64>().unwrap();
        let plain = crate::deps::Zeroizing::new(crate::deps::serde_json::to_string(&crate::deps::serde_json::json!({ "token": session.token, "clienttime": time })).unwrap());
        let encrypted = rsablk(plain.as_bytes())?;
        let body = crate::deps::serde_json::json!({ "visit_time": time, "usertype": 1, "p": encrypted, "userid": session.user_id });
        params.insert("plat".into(), "1".into());
        self.req("https://gateway.kugou.com/v3/get_my_info", params, Some(&body), false, "usercenter.kugou.com")
    }

    /// 搜索单曲列表
    pub fn srch(&self, qry: &str, pg: u32, lmt: u32) -> Result<SearchSongResp, AppError> {
        let mut params = self.params();
        for (key, value) in [("albumhide", "0"), ("iscorrection", "1"), ("keyword", qry), ("nocollect", "0"), ("platform", "AndroidFilter")] { params.insert(key.into(), value.into()); }
        params.insert("page".into(), pg.to_string());
        params.insert("pagesize".into(), lmt.to_string());
        let root = self.req("https://gateway.kugou.com/v3/search/song", params, None, false, "complexsearch.kugou.com")?;
        let data: SearchSongResp = crate::deps::serde_json::from_value(root)
            .map_err(|_| NetworkError::Server("搜索响应格式不匹配".into()))?;
        if data.status != 1 || data.err_code != 0 {
            return Err(NetworkError::Server(format!("搜索失败：{}", data.err_code)).into());
        }
        Ok(data)
    }

    /// 根据哈希取音频播放地址
    pub fn trkurl(&self, hash: &str, quality: AudioQuality) -> Result<Value, AppError> {
        if !self.sess().dfid_registered {
            let _ = self.regdev();
        }
        let session = self.sess();
        let mut p = self.params();
        let hash = hash.to_lowercase();
        let quality = quality.kgstr();
        // 原生song_url参数。
        for (key, value) in [
            ("hash", hash.as_str()), ("appid", "3116"), ("clientver", "11430"),
            ("mid", session.mid.as_str()), ("dfid", session.dfid.as_str()), ("uuid", "-"),
            ("album_id", "0"), ("area_code", "1"), ("ssa_flag", "is_fromtrack"),
            ("version", "11430"), ("page_id", "967177915"), ("quality", quality),
            ("album_audio_id", "0"), ("behavior", "play"), ("pid", "411"),
            ("cmd", "26"), ("pidversion", "3001"), ("IsFreePart", "0"),
            ("ppage_id", "356753938"), ("cdnBackup", "1"), ("module", ""),
        ] { p.insert(key.to_string(), value.to_string()); }
        let time = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs().to_string();
        p.insert("clienttime".into(), time.clone());
        p.insert("key".into(), Self::mdhex(format!("{hash}{KEY_SALT}3116{}{}", session.mid, session.user_id).as_bytes()));
        self.req("https://gateway.kugou.com/v5/url", p, None, false, "trackercdn.kugou.com")
    }

    /// 根据哈希取公开歌曲基础信息（歌名/歌手/专辑/时长/封面）。
    pub(crate) fn trkinf(&self, hash: &str) -> Result<Value, AppError> {
        let mut params = BTreeMap::new();
        params.insert("cmd".into(), "playInfo".into());
        params.insert("hash".into(), hash.to_lowercase());
        self.req("https://m.kugou.com/app/i/getSongInfo.php", params, None, false, "")
    }

    /// 获取每日推荐歌曲列表。
    pub(crate) fn dailyrec(&self) -> Result<Value, AppError> {
        let mut params = self.params();
        params.insert("platform".into(), "ios".into());
        self.reqraw("https://gateway.kugou.com/everyday_song_recommend", params, Some(String::new()), false, "everydayrec.service.kugou.com", "application/json")
    }

    /// 获取广场推荐歌单列表。
    pub(crate) fn plzpl(&self, page: u32, pagesize: u32) -> Result<Value, AppError> {
        let session = self.sess();
        let params = self.params();
        let time = params["clienttime"].clone();
        let key = Self::mdhex(format!("3116{SALT_LITE}11440{time}").as_bytes());
        let body = crate::deps::serde_json::json!({
            "appid": 3116,
            "mid": session.mid,
            "clientver": 11440,
            "platform": "android",
            "clienttime": time,
            "userid": session.user_id,
            "module_id": 1,
            "page": page,
            "pagesize": pagesize,
            "key": key,
            "special_recommend": {
                "withtag": 1, "withsong": 1, "sort": 1, "ugc": 1,
                "is_selected": 0, "withrecommend": 1, "area_code": 1, "categoryid": 0
            },
            "req_multi": 1,
            "retrun_min": 5,
            "return_special_falg": 1
        });
        self.req("https://gateway.kugou.com/v2/special_recommend", params, Some(&body), false, "specialrec.service.kugou.com")
    }

    /// 分页读取公开歌单曲目。
    pub(crate) fn pubpltrks(&self, global_id: &str, begin_idx: usize, pagesize: u32) -> Result<Value, AppError> {
        let mut params = self.params();
        for (k, v) in [
            ("area_code", "1"), ("plat", "1"), ("type", "1"), ("mode", "1"),
            ("personal_switch", "1"), ("extend_fields", "abtags,hot_cmt,popularization"),
            ("pagesize", &pagesize.to_string()), ("global_collection_id", global_id),
            ("begin_idx", &begin_idx.to_string()),
        ] {
            params.insert(k.into(), v.into());
        }
        self.req("https://gateway.kugou.com/pubsongs/v2/get_other_list_file_nofilt", params, None, false, "")
    }

    /// 读取专辑信息与封面。
    pub(crate) fn albdtl(&self, album_id: &str) -> Result<Value, AppError> {
        let body = crate::deps::serde_json::json!({
            "data": [{ "album_id": album_id }],
            "is_buy": 0,
            "fields": "album_id,album_name,sizable_cover,type,author_name"
        });
        self.req("https://gateway.kugou.com/kmr/v2/albums", self.params(), Some(&body), false, "openapi.kugou.com")
    }

    /// 分页读取专辑曲目列表。
    pub(crate) fn albsngs(&self, album_id: &str, page: u32, pagesize: u32) -> Result<Value, AppError> {
        let body = crate::deps::serde_json::json!({
            "album_id": album_id,
            "is_buy": "",
            "page": page,
            "pagesize": pagesize
        });
        self.req("https://gateway.kugou.com/v1/album_audio/lite", self.params(), Some(&body), false, "openapi.kugou.com")
    }

    /// 查询曲目各音质可用权限与专属哈希。
    pub(crate) fn prvlite(&self, hashes: &[String]) -> Result<Value, AppError> {
        let mut seen = std::collections::HashSet::new();
        let mut resource = Vec::new();
        for hash in hashes {
            let low = hash.trim().to_ascii_lowercase();
            if low.is_empty() || !seen.insert(low.clone()) { continue; }
            let album_id = if resource.is_empty() { Value::String(String::new()) } else { Value::from(0) };
            resource.push(crate::deps::serde_json::json!({
                "type": "audio", "page_id": 0, "hash": low, "album_id": album_id
            }));
        }
        let body = crate::deps::serde_json::json!({
            "appid": 3116,
            "area_code": 1,
            "behavior": "play",
            "clientver": 11440,
            "need_hash_offset": 1,
            "relate": 1,
            "support_verify": 1,
            "resource": resource,
            "qualities": ["128", "320", "flac", "high", "viper_atmos", "viper_tape", "viper_clear", "super", "multitrack"]
        });
        self.req("https://gateway.kugou.com/v2/get_res_privilege/lite", self.params(), Some(&body), false, "media.store.kugou.com")
    }

    /// 创建云端用户歌单。
    pub(crate) fn mkpl(&self, name: &str) -> Result<Value, AppError> {
        let session = self.sess();
        if session.user_id == 0 || session.token.is_empty() {
            return Err(crate::domain::error::AuthError::Missing.into());
        }
        let body = crate::deps::serde_json::json!({
            "userid": session.user_id.to_string(),
            "token": session.token,
            "total_ver": 0,
            "name": name,
            "type": "0",
            "source": 1,
            "is_pri": 0,
            "list_create_gid": "",
            "from_shupinmv": 0
        });
        self.req("https://gateway.kugou.com/cloudlist.service/v5/add_list", self.params(), Some(&body), false, "")
    }

    /// 删除云端自建歌单（加密体协议，参考原生 /v2/delete_list；响应同为加密体）。
    pub(crate) fn dellst(&self, list_id: i64) -> Result<(), AppError> {
        let session = self.sess();
        if session.user_id == 0 || session.token.is_empty() {
            return Err(crate::domain::error::AuthError::Missing.into());
        }
        let clienttime = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs().to_string();
        use crate::deps::rand::Rng;
        let plain = crate::deps::serde_json::to_string(&crate::deps::serde_json::json!({
            "listid": list_id,
            "total_ver": 0,
            "type": 1,
        })).map_err(|error| NetworkError::Server(format!("序列化待删歌单失败：{error}")))?;
        let mut rng = crate::deps::rand::thread_rng();
        let aes_key = Self::aeskey(&mut rng);
        let body = aesenc(&aes_key, plain.as_bytes())?;
        let rsa_in = crate::deps::serde_json::to_string(&crate::deps::serde_json::json!({
            "aes": aes_key, "uid": session.user_id.to_string(), "token": session.token,
        })).map_err(|error| NetworkError::Server(format!("序列化删除凭据失败：{error}")))?;
        let mut fill = [0u8; 64];
        rng.fill(&mut fill);
        let p = pkcs1blk(rsa_in.as_bytes(), &fill)?.to_uppercase();
        let key = Self::mdhex(format!("3116{SALT_LITE}11440{clienttime}").as_bytes());
        let mut params = self.params();
        params.insert("key".into(), key);
        params.insert("last_area".into(), "gztx".into());
        params.insert("last_time".into(), clienttime.clone());
        params.insert("p".into(), p);
        params.insert("signature".into(), Self::sgn(&params, &body, false));
        let mut request = self.http.post("https://gateway.kugou.com/v2/delete_list")
            .query(&params)
            .header("User-Agent", USER_AGENT)
            .header("mid", &session.mid).header("dfid", &session.dfid)
            .header("x-router", "cloudlist.service.kugou.com")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(body);
        if let Some(time) = params.get("clienttime") { request = request.header("clienttime", time); }
        let response = request.send().map_err(neterr)?.error_for_status().map_err(neterr)?;
        if response.content_length().is_some_and(|size| size > JSON_LIMIT) {
            return Err(NetworkError::Server("删除歌单响应超过大小限制".into()).into());
        }
        let mut raw = Vec::new();
        response.take(JSON_LIMIT + 1).read_to_end(&mut raw).map_err(|_| NetworkError::Server("读取删除响应失败".into()))?;
        // 解密成功即删除成功。
        aesdec(&aes_key, &raw)?;
        Ok(())
    }

    /// 向云端歌单追加曲目条目。
    pub(crate) fn addpltrks(&self, list_id: i64, entries: &[Value]) -> Result<Value, AppError> {
        let session = self.sess();
        if session.user_id == 0 || session.token.is_empty() {
            return Err(crate::domain::error::AuthError::Missing.into());
        }
        let mut params = self.params();
        params.insert("last_time".into(), params["clienttime"].clone());
        params.insert("last_area".into(), "gztx".into());
        let body = crate::deps::serde_json::json!({
            "userid": session.user_id.to_string(),
            "token": session.token,
            "listid": list_id.to_string(),
            "list_ver": 0,
            "type": 0,
            "slow_upload": 1,
            "scene": "false;null",
            "data": entries
        });
        self.req("https://gateway.kugou.com/cloudlist.service/v6/add_song", params, Some(&body), false, "")
    }

    /// 计算协议所需MD5值。
    pub(crate) fn mdhex(bytes: &[u8]) -> String {
        use crate::deps::md5::{Digest, Md5};
        format!("{:x}", Md5::digest(bytes))
    }

    /// 生成十六进制设备标识。
    fn newguid() -> String {
        Self::mdhex(crate::deps::uuid::Uuid::new_v4().to_string().as_bytes())
    }

    /// 生成六位随机AES密钥。
    fn aeskey<R: crate::deps::rand::Rng + ?Sized>(rng: &mut R) -> String {
        const POOL: &[u8] = b"1234567890abcdefghijklmnopqrstuvwxyz";
        (0..6).map(|_| POOL[rng.gen_range(0..POOL.len())] as char).collect()
    }
}

/// 复用参考原生协议的1024位裸RSA右侧补零规则。
fn rsablk(bytes: &[u8]) -> Result<String, AppError> {
    if bytes.len() > 128 { return Err(NetworkError::Server("用户资料凭据长度超出协议限制".into()).into()); }
    let mut block = crate::deps::Zeroizing::new([0u8; 128]);
    block[..bytes.len()].copy_from_slice(bytes);
    Ok(rsaenc(&block[..]).to_uppercase())
}

/// 对128字节块执行裸RSA加密并输出256位十六进制。
fn rsaenc(block: &[u8]) -> String {
    let modulus = crate::deps::BigUint::parse_bytes(LITE_MOD.as_bytes(), 16).unwrap();
    let encrypted = crate::deps::BigUint::from_bytes_be(block).modpow(&crate::deps::BigUint::from(65537u32), &modulus);
    format!("{:0>256}", encrypted.to_str_radix(16))
}

/// 复用参考原生协议的 PKCS#1 v1.5 块加密规则。
fn pkcs1blk(data: &[u8], fill: &[u8]) -> Result<String, AppError> {
    const MAX: usize = 128 - 11;
    if data.len() > MAX || fill.is_empty() {
        return Err(NetworkError::Server("设备注册RSA参数异常".into()).into());
    }
    let pad_len = 128 - 3 - data.len();
    let mut block = crate::deps::Zeroizing::new(Vec::with_capacity(128));
    block.push(0x00);
    block.push(0x02);
    let mut cursor = 0usize;
    let mut written = 0usize;
    let mut scanned = 0usize;
    while written < pad_len {
        if scanned >= fill.len() * 256 {
            return Err(NetworkError::Server("设备注册随机填充不可用".into()).into());
        }
        let b = fill[cursor % fill.len()];
        cursor += 1;
        scanned += 1;
        if b != 0 {
            block.push(b);
            written += 1;
        }
    }
    block.push(0x00);
    block.extend_from_slice(data);
    Ok(rsaenc(&block[..]))
}

/// 派生 AES-128-CBC 密钥与 IV。
fn aesmat(key: &str) -> ([u8; 16], [u8; 16]) {
    let digest = KugouClient::mdhex(key.as_bytes());
    let mut k = [0u8; 16];
    let mut iv = [0u8; 16];
    k.copy_from_slice(&digest.as_bytes()[..16]);
    iv.copy_from_slice(&digest.as_bytes()[16..32]);
    (k, iv)
}

/// AES-128-CBC + PKCS#7 加密为 base64。
fn aesenc(key: &str, plain: &[u8]) -> Result<String, AppError> {
    use crate::deps::aes::{Aes128, cipher::{BlockEncrypt, KeyInit, generic_array::GenericArray}};
    use crate::deps::base64::Engine;
    let (k, iv) = aesmat(key);
    let pad = 16 - (plain.len() % 16);
    let mut buf = Vec::with_capacity(plain.len() + pad);
    buf.extend_from_slice(plain);
    buf.extend(std::iter::repeat_n(pad as u8, pad));
    let cipher = Aes128::new(GenericArray::from_slice(&k));
    let mut prev = iv;
    for chunk in buf.chunks_mut(16) {
        for (b, c) in chunk.iter_mut().zip(prev.iter()) { *b ^= c; }
        cipher.encrypt_block(GenericArray::from_mut_slice(chunk));
        prev.copy_from_slice(chunk);
    }
    Ok(crate::deps::base64::engine::general_purpose::STANDARD.encode(&buf))
}

/// AES-128-CBC 解密原始字节。
fn aesdec(key: &str, cipher_bytes: &[u8]) -> Result<Vec<u8>, AppError> {
    use crate::deps::aes::{Aes128, cipher::{BlockDecrypt, KeyInit, generic_array::GenericArray}};
    if cipher_bytes.is_empty() || cipher_bytes.len() % 16 != 0 {
        return Err(NetworkError::Server("AES响应长度无效".into()).into());
    }
    let (k, iv) = aesmat(key);
    let cipher = Aes128::new(GenericArray::from_slice(&k));
    let mut prev = iv;
    let mut out = Vec::with_capacity(cipher_bytes.len());
    for chunk in cipher_bytes.chunks(16) {
        let mut blk = GenericArray::clone_from_slice(chunk);
        cipher.decrypt_block(&mut blk);
        for (b, c) in blk.iter_mut().zip(prev.iter()) { *b ^= c; }
        out.extend_from_slice(&blk);
        prev.copy_from_slice(chunk);
    }
    let pad = usize::from(*out.last().unwrap_or(&0));
    if pad > 0 && pad <= 16 && pad <= out.len() { out.truncate(out.len() - pad); }
    Ok(out)
}

/// 去除可能携带授权参数的错误URL。
fn neterr(error: crate::deps::reqwest::Error) -> AppError {
    if error.is_timeout() { return NetworkError::Timeout.into(); }
    if error.is_connect() { return NetworkError::Offline.into(); }
    NetworkError::Server(error.without_url().to_string()).into()
}

/// 有界读取JSON响应并校验HTTP状态。
fn rdjson(response: Response) -> Result<Value, AppError> {
    let response = response.error_for_status().map_err(neterr)?;
    if response.content_length().is_some_and(|size| size > JSON_LIMIT) {
        return Err(NetworkError::Server("响应超过大小限制".into()).into());
    }
    let mut bytes = Vec::new();
    response.take(JSON_LIMIT + 1).read_to_end(&mut bytes)
        .map_err(|_| NetworkError::Server("读取响应失败".into()))?;
    if bytes.len() as u64 > JSON_LIMIT {
        return Err(NetworkError::Server("响应超过大小限制".into()).into());
    }
    crate::deps::serde_json::from_slice(&bytes)
        .map_err(|_| NetworkError::Server("服务端未返回有效JSON".into()).into())
}

impl Default for KugouClient {
    /// 构造默认酷狗客户端。
    fn default() -> Self {
        Self::nw()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// 对照参考仓库的酷狗概念版RSA与AES已知答案。
    #[test]
    fn tst_liters() {
        assert_eq!(rsablk(b"hello world").unwrap().to_lowercase(), "4907d21b3d98a308858cdf4c753018ef5f9da7f07c9b40460041006c28e255b80bdbc98b3d26cf4a8692fb84e73c56ab92149504e7130e51832276f327c0570698059d4adfbe287d42b01a3ee119c27630ff9d5a2e66f08f77a3ddef6c1a151afbb80db7473b6d65f9516e826ac3299bcf68aec0461cfa52940d3068ec524374");
        assert!(rsablk(&[0; 129]).is_err());
        let fill: [u8; 16] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
        assert_eq!(pkcs1blk(b"hello world", &fill).unwrap(), "9025398f6f9b6f61887303d23c62cc46c63125377042927bd6040648b524ffd663fdfd0088fc0ff3a73eff65d96aa7b3730e442c5729ca291170b6c81646b66a4991e6a693251cfdacd35fe96ca46b82ad6ab6e01bb83a18ddec0149df4a6b34ae6c774e5f4e016ec64135de0239f1b01420807b86343d7bc7967b5ad49654c4");
        assert_eq!(aesenc("15iw0r", r#"{"hello":"world","中文":"值","n":42,"nested":{"a":[1,2]}}"#.as_bytes()).unwrap(), "NT568O7l3E83PNQy5gEbv6YZ3dXr7r2B0Xb7NmnWbZlBvfBfeJDec4hGNcv09JvgjGPxUBUWwAPxNmOHpwsiJg==");
        use crate::deps::base64::Engine;
        let ct = crate::deps::base64::engine::general_purpose::STANDARD.decode("02H1lHOQIzwMrj05HOAgLvLiPkqf9yl7uV+kZlcfSDuaw7BimABc+k0W8KH/NWgYcBAQu8TiWtQYtmsE6ZXj7g==").unwrap();
        assert_eq!(String::from_utf8(aesdec("1jx5zx", &ct).unwrap()).unwrap(), r#"{"status":1,"data":{"dfid":"DFIDFIXTURE0123456789ab"}}"#);
    }

    /// 验证加密会话保存、设备稳定和损坏文件不覆盖。
    #[test]
    fn tst_session() {
        let root = std::env::temp_dir().join(format!("kodog-session-{}", crate::deps::uuid::Uuid::new_v4()));
        let path = root.join("secrets.dat");
        let client = KugouClient::opn(&path).unwrap();
        let mut session = client.sess();
        let mid = session.mid.clone();
        session.user_id = 42;
        session.token = "synthetic-credential".into();
        client.svsess(session).unwrap();
        let restored = KugouClient::opn(&path).unwrap();
        assert_eq!(restored.sess().user_id, 42);
        assert_eq!(restored.sess().token, "synthetic-credential");
        assert_eq!(restored.sess().mid, mid);
        let mut bytes = std::fs::read(&path).unwrap();
        *bytes.last_mut().unwrap() ^= 1;
        std::fs::write(&path, &bytes).unwrap();
        assert!(KugouClient::opn(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(root).unwrap();
    }

    /// 只读核对真实音源响应字段，不输出播放地址与凭据。
    #[test]
    #[ignore = "需要用户已经扫码登录"]
    fn tst_mednet() {
        let path = std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap()).join("KoDogLite/secrets.dat");
        let client = KugouClient::opn(path).unwrap();
        let root = client.trkurl("b3a52a7a958bf0aed0ebfba2e9a818b7", AudioQuality::High).unwrap();
        let data = root.get("data").unwrap_or(&root);
        let data = data.as_array().and_then(|items| items.first()).unwrap_or(data);
        eprintln!("media root fields: {:?}", root.as_object().map(|root| root.keys().collect::<Vec<_>>()));
        eprintln!("media data fields: {:?}", data.as_object().map(|root| root.keys().collect::<Vec<_>>()));
        eprintln!("media quality {:?}, bitrate {:?}, extension {:?}", data.get("q"), data.get("bitRate"), data.get("extName"));
        assert!(data.get("url").is_some());
        for quality in [AudioQuality::Standard, AudioQuality::Lossless] {
            match client.trkurl("b3a52a7a958bf0aed0ebfba2e9a818b7", quality) {
                Ok(root) => eprintln!("requested {:?}, bitrate {:?}, extension {:?}", quality, root.get("bitRate"), root.get("extName")),
                Err(error) => eprintln!("requested {:?}, result {}", quality, error),
            }
        }
    }

    /// 核对原生搜索是否提供封面与完整曲目字段。
    #[test]
    #[ignore = "需要访问官方搜索接口"]
    fn tst_ntvsrch() {
        let client = KugouClient::nw();
        let parsed = client.srch("晴天", 1, 2).unwrap();
        let tracks = parsed.data.unwrap().lists;
        assert!(!tracks.is_empty());
        assert!(tracks.iter().all(|track| !track.song_name.is_empty() && !track.file_hash.is_empty()));
        assert!(tracks.iter().any(|track| track.image.as_ref().is_some_and(|image| image.starts_with("http"))));
    }

    /// 验证官方歌词搜索、下载和真实时间轴。
    #[test]
    #[ignore = "需要访问官方歌词接口"]
    fn tst_lyrnet() {
        let text = KugouClient::nw().lyrtxt("b3a52a7a958bf0aed0ebfba2e9a818b7").unwrap();
        let lines = crate::services::lyrics::prslyrc(&text);
        assert!(lines.len() > 5);
        assert!(lines.windows(2).all(|pair| pair[0].begin_ms <= pair[1].begin_ms));
        eprintln!("verified lyric timeline entries: {}", lines.len());
    }

    /// 验证状态、格式及已知和未知响应长度的读取上限。
    #[test]
    fn tst_rdjson() {
        let mut oversized = vec![b' '; JSON_LIMIT as usize + 1];
        oversized[0] = b'"';
        *oversized.last_mut().unwrap() = b'"';
        for (status, body, length, valid) in [
            ("200 OK", b"{\"status\":1}".to_vec(), true, true),
            ("503 Unavailable", b"{}".to_vec(), true, false),
            ("200 OK", b"not json".to_vec(), true, false),
            ("200 OK", oversized.clone(), true, false),
            ("200 OK", oversized, false, false),
        ] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let server = std::thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                socket.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
                let mut request = [0; 2048];
                let _ = socket.read(&mut request).unwrap();
                let size = if length { format!("Content-Length: {}\r\n", body.len()) } else { String::new() };
                let header = format!("HTTP/1.1 {status}\r\n{size}Connection: close\r\n\r\n");
                let _ = socket.write_all(header.as_bytes());
                let _ = socket.write_all(&body);
            });
            let response = Client::builder().no_proxy().timeout(std::time::Duration::from_secs(5))
                .build().unwrap().get(format!("http://{address}")).send().unwrap();
            assert_eq!(rdjson(response).is_ok(), valid);
            server.join().unwrap();
        }
    }
}
