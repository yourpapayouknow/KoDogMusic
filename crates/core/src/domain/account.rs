use crate::deps::serde::{Deserialize, Serialize};

/// 签到状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum CheckInState {
    #[default]
    Available,
    Checked,
    Pending,
}

/// 用户账户状态
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AccountState {
    #[default]
    Guest,
    Authorized {
        user_id: String,
        display_name: String,
        avatar_ref: Option<String>,
        vip_type: u32,
    },
}

/// 官方二维码授权状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum QrStage { #[default] Waiting, Confirming, Authorized, Expired }

/// 当前授权二维码，不含用户令牌。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginQr { pub key: String, pub link: String, pub stage: QrStage }
