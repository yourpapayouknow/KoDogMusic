// 二维码登录与签到适配。
use crate::application::ports::AccountPort;
use crate::domain::account::{AccountState, CheckInState, LoginQr, QrStage};
use crate::domain::error::{AppError, AuthError, NetworkError};
use crate::provider::kugou::client::KugouClient;
use crate::deps::serde_json::Value;

pub struct KugouAuth { client: KugouClient, checkin: std::sync::Mutex<(String, CheckInState, bool)> }

/// 跨日时重置当天的签到缓存。
fn rollday(state: &mut (String, CheckInState, bool), day: &str) {
    if state.0 != day { *state = (day.to_string(), CheckInState::Available, false); }
}

impl KugouAuth {
    /// 共享音源和账户使用的同一份凭据。
    pub fn nw(client: KugouClient) -> Self { Self { client, checkin: std::sync::Mutex::new((super::checkin::today(), CheckInState::Available, false)) } }

    /// 更新真实昵称与头像并加密保存。
    pub fn rfrsh(&self) -> Result<(), AppError> {
        let root = self.client.usrdtl()?;
        let data = root.get("data").unwrap_or(&root);
        let nickname = data.get("nickname").and_then(Value::as_str).filter(|value| !value.is_empty())
            .ok_or_else(|| NetworkError::Server("用户资料缺少昵称".into()))?;
        let mut session = self.client.sess();
        session.nickname = nickname.chars().take(128).collect();
        session.avatar = data.get("pic").and_then(Value::as_str).filter(|value| !value.is_empty()).map(ToString::to_string);
        self.client.svsess(session)
    }
}

impl AccountPort for KugouAuth {
    /// 根据真实授权与已保存资料呈现账户。
    fn stt(&self) -> AccountState {
        let session = self.client.sess();
        if session.user_id == 0 || session.token.is_empty() { return AccountState::Guest; }
        AccountState::Authorized { user_id: session.user_id.to_string(),
            display_name: if session.nickname.is_empty() { format!("ID {}", session.user_id) } else { session.nickname },
            avatar_ref: session.avatar, vip_type: 0 }
    }

    /// 请求官方登录挑战并生成授权页面地址。
    fn qrstrt(&self) -> Result<LoginQr, AppError> {
        let mut params = self.client.params();
        for (key, value) in [("appid", "1001"), ("type", "1"), ("plat", "4"), ("srcappid", "2919"),
            ("qrcode_txt", "https://h5.kugou.com/apps/loginQRCode/html/index.html?appid=3116&")] {
            params.insert(key.into(), value.into());
        }
        let root = self.client.req("https://login-user.kugou.com/v2/qrcode", params, None, true, "")?;
        let key = root.get("data").and_then(|data| data.get("qrcode")).and_then(Value::as_str)
            .filter(|key| !key.is_empty() && key.len() <= 1024).ok_or_else(|| NetworkError::Server("未取得登录二维码".into()))?;
        let mut link = crate::deps::url::Url::parse("https://h5.kugou.com/apps/loginQRCode/html/index.html").unwrap();
        link.query_pairs_mut().append_pair("qrcode", key);
        Ok(LoginQr { key: key.to_string(), link: link.into(), stage: QrStage::Waiting })
    }

    /// 仅在官方返回授权成功后保存用户令牌。
    fn qrpoll(&self, key: &str) -> Result<QrStage, AppError> {
        if key.is_empty() || key.len() > 1024 { return Err(AuthError::Expired.into()); }
        let mut params = self.client.params();
        for (name, value) in [("plat", "4"), ("appid", "3116"), ("srcappid", "2919"), ("qrcode", key)] { params.insert(name.into(), value.into()); }
        let root = self.client.req("https://login-user.kugou.com/v2/get_userinfo_qrcode", params, None, true, "")?;
        let data = root.get("data").unwrap_or(&root);
        match data.get("status").or_else(|| data.get("code")).and_then(Value::as_i64) {
            Some(0) => Ok(QrStage::Expired),
            Some(1) => Ok(QrStage::Waiting),
            Some(2) => Ok(QrStage::Confirming),
            Some(4) => {
                let user_id = data.get("userid").and_then(|value| value.as_u64().or_else(|| value.as_str().and_then(|value| value.parse().ok())))
                    .filter(|value| *value > 0).ok_or(AuthError::Denied)?;
                let token = data.get("token").and_then(Value::as_str).filter(|value| !value.is_empty() && value.len() <= 1024).ok_or(AuthError::Denied)?;
                let mut session = self.client.sess();
                session.user_id = user_id;
                session.token = token.to_string();
                session.nickname.clear();
                session.avatar = None;
                self.client.svsess(session)?;
                self.rfrsh()?;
                Ok(QrStage::Authorized)
            }
            _ => Err(NetworkError::Server("无法识别登录状态".into()).into()),
        }
    }

    /// 每会话按日向服务器核对一次签到状态。
    fn synck(&self) {
        let day = super::checkin::today();
        {
            let mut state = self.checkin.lock().unwrap();
            rollday(&mut state, &day);
            if state.2 || self.client.sess().user_id == 0 { return; }
            state.2 = true;
        }
        match super::checkin::CheckInService::stt(&self.client, &day) {
            Ok(checked) => *self.checkin.lock().unwrap() = (day, checked, true),
            // 查询失败允许重试。
            Err(_) => self.checkin.lock().unwrap().2 = false,
        }
    }

    /// 返回本地缓存的签到结果。
    fn chkstt(&self) -> CheckInState {
        let mut state = self.checkin.lock().unwrap();
        let day = super::checkin::today();
        rollday(&mut state, &day);
        state.1
    }

    /// 手动领取并仅保存服务端确认的结果。
    fn dochk(&self) -> Result<CheckInState, AppError> {
        let day = super::checkin::today();
        let state = super::checkin::CheckInService::dochk(&self.client, &day)?;
        *self.checkin.lock().unwrap() = (day, state, true);
        Ok(state)
    }

    /// 保留设备标识并移除本地授权。
    fn lgout(&self) -> Result<(), AppError> {
        let mut session = self.client.sess();
        session.user_id = 0;
        session.token.clear();
        session.nickname.clear();
        session.avatar = None;
        self.client.svsess(session)?;
        *self.checkin.lock().unwrap() = (super::checkin::today(), CheckInState::Available, false);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证真实二维码契约，不读取或授权用户账户。
    #[test]
    #[ignore = "需要访问官方二维码接口"]
    fn tst_qrnet() {
        let auth = KugouAuth::nw(KugouClient::nw());
        let qr = auth.qrstrt().unwrap();
        assert!(!qr.key.is_empty());
        assert!(qr.link.starts_with("https://h5.kugou.com/"));
        assert_eq!(auth.qrpoll(&qr.key).unwrap(), QrStage::Waiting);
        assert_eq!(auth.stt(), AccountState::Guest);
    }
}
