use crate::domain::account::CheckInState;
use crate::domain::error::{AppError, AuthError, NetworkError};
use crate::provider::kugou::client::KugouClient;
use crate::deps::{serde_json::Value, win::{GetLocalTime, SYSTEMTIME}};

/// 每日签到服务
pub struct CheckInService;

impl CheckInService {
    /// 读取服务器当天的实际领取记录。
    pub fn stt(client: &KugouClient, day: &str) -> Result<CheckInState, AppError> {
        if client.sess().user_id == 0 { return Err(AuthError::Missing.into()); }
        let mut params = client.params();
        params.insert("latest_limit".into(), "100".into());
        let root = client.req("https://gateway.kugou.com/youth/v1/activity/get_month_vip_record", params, None, false, "")?;
        chkrec(&root, day)
    }

    /// 仅响应手动点击领取，并读取记录确认结果。
    pub fn dochk(client: &KugouClient, day: &str) -> Result<CheckInState, AppError> {
        if Self::stt(client, day)? == CheckInState::Checked { return Ok(CheckInState::Checked); }
        let mut params = client.params();
        params.insert("source_id".into(), "90139".into());
        params.insert("receive_day".into(), day.into());
        client.reqraw("https://gateway.kugou.com/youth/v1/recharge/receive_vip_listen_song", params, Some(String::new()), false, "", "application/x-www-form-urlencoded")?;
        for _ in 0..5 {
            if Self::stt(client, day)? == CheckInState::Checked { return Ok(CheckInState::Checked); }
            std::thread::sleep(std::time::Duration::from_millis(400));
        }
        Err(NetworkError::Server("尚未确认当天领取成功，请查看官方账号状态".into()).into())
    }
}

/// 返回本地日期的 YYYY-MM-DD 文本。
pub fn today() -> String {
    let mut time = SYSTEMTIME::default();
    unsafe { GetLocalTime(&mut time); }
    format!("{:04}-{:02}-{:02}", time.wYear, time.wMonth, time.wDay)
}

/// 解析当日明确领取成功的服务器记录。
fn chkrec(root: &Value, day: &str) -> Result<CheckInState, AppError> {
    let rows = root.get("data").and_then(|data| data.get("list")).and_then(Value::as_array)
        .ok_or_else(|| NetworkError::Server("签到记录格式无效".into()))?;
    Ok(if rows.iter().any(|row| row.get("day").and_then(Value::as_str) == Some(day) && row.get("receive_vip").and_then(Value::as_i64) == Some(1)) {
        CheckInState::Checked
    } else { CheckInState::Available })
}

#[cfg(test)]
mod tests {
    use super::*;
    /// 验证跨日、未领取与非法响应不显示成功。
    #[test]
    fn tst_chkrec() {
        let root = crate::deps::serde_json::json!({"data":{"list":[{"day":"2026-11-01","receive_vip":1},{"day":"2026-11-02","receive_vip":0}]}});
        assert_eq!(chkrec(&root, "2026-11-01").unwrap(), CheckInState::Checked);
        assert_eq!(chkrec(&root, "2026-11-02").unwrap(), CheckInState::Available);
        assert!(chkrec(&Value::Null, "2026-11-02").is_err());
        assert_eq!(today().len(), 10);
    }

    /// 只读核对当前账号的签到状态，不领取奖励。
    #[test]
    #[ignore = "需要用户已经扫码登录，仅查询状态"]
    fn tst_chknet() {
        let root = std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap()).join("KoDogLite/secrets.dat");
        let client = KugouClient::opn(root).unwrap();
        println!("签到只读状态: {:?}", CheckInService::stt(&client, &today()).unwrap());
    }
}
