use crate::domain::account::{AccountState, CheckInState};
use crate::domain::error::{AppError, CatalogError};
use crate::domain::ids::TrackId;
use crate::domain::playback::PlaybackProgress;
use crate::domain::settings::AppSettings;
use crate::domain::track::{AudioQuality, CoverDeck, DownloadedFile, LyricLine, ResolvedSource, Track, TrackSummary, UserPlaylistMeta};

/// 媒体目录与元数据端口
pub trait CatalogPort: Send + Sync {
    /// 按关键字搜索单曲
    fn srchtrks(&self, qry: &str, pg: u32, lmt: u32) -> Result<Vec<TrackSummary>, AppError>;
    /// 获取指定曲目的详细元数据
    fn trkdtl(&self, id: &TrackId) -> Result<Track, AppError>;
    /// 获取指定曲目的歌词列表
    fn lyrc(&self, id: &TrackId) -> Result<Vec<LyricLine>, AppError>;
    /// 获取指定曲目的原始LRC文本
    fn lyrtxt(&self, _id: &TrackId) -> Result<String, AppError> { Ok(String::new()) }
    /// 获取每日推荐歌曲列表。
    fn dailyrec(&self) -> Result<Vec<TrackSummary>, AppError> { Ok(Vec::new()) }
    /// 获取上游推荐歌单列表。
    fn recpl(&self, _lmt: u32) -> Result<Vec<UserPlaylistMeta>, AppError> { Ok(Vec::new()) }
    /// 获取公开歌单全部歌曲。
    fn pubpltrks(&self, _gid: &str) -> Result<Vec<TrackSummary>, AppError> { Ok(Vec::new()) }
    /// 获取专辑详情与全部曲目。
    fn albdtl(&self, _aid: &str) -> Result<(String, Option<String>, Vec<TrackSummary>), AppError> {
        Err(CatalogError::NotFound.into())
    }
    /// 查询曲目在上游实际提供的下载品质选项。
    fn prvqlt(&self, _trks: &[TrackSummary]) -> Result<Vec<AudioQuality>, AppError> {
        Ok(vec![AudioQuality::High])
    }
}

/// 用户账户与鉴权端口
pub trait AccountPort: Send + Sync {
    /// 获取官方授权二维码。
    fn qrstrt(&self) -> Result<crate::domain::account::LoginQr, AppError> { Err(crate::domain::error::AuthError::Missing.into()) }
    /// 查询二维码的实际授权状态。
    fn qrpoll(&self, _key: &str) -> Result<crate::domain::account::QrStage, AppError> { Err(crate::domain::error::AuthError::Missing.into()) }
    /// 获取账户状态
    fn stt(&self) -> AccountState;
    /// 获取签到状态
    fn chkstt(&self) -> CheckInState;
    /// 每会话按日向服务器核对一次签到状态
    fn synck(&self);
    /// 执行每日签到
    fn dochk(&self) -> Result<CheckInState, AppError>;
    /// 退出登录
    fn lgout(&self) -> Result<(), AppError>;
}

/// 收藏与用户媒体库端口
pub trait LibraryPort: Send + Sync {
    /// 获取收藏曲目列表
    fn fvrts(&self) -> Result<Vec<TrackSummary>, AppError>;
    /// 切换收藏状态
    fn tglfvrt(&self, track: &TrackSummary) -> Result<bool, AppError>;
    /// 检查是否已收藏
    fn isfvrt(&self, id: &TrackId) -> Result<bool, AppError>;
    /// 获取账号下全部歌单列表。
    fn usrpls(&self) -> Result<Vec<UserPlaylistMeta>, AppError> { Ok(Vec::new()) }
    /// 读取指定账号歌单全部曲目。
    fn pltrks(&self, _lid: i64) -> Result<Vec<TrackSummary>, AppError> { Ok(Vec::new()) }
    /// 新建用户歌单并返回歌单信息。
    fn mkpl(&self, _nm: &str) -> Result<UserPlaylistMeta, AppError> {
        Err(crate::domain::error::AuthError::Missing.into())
    }
    /// 向指定账号歌单批量追加曲目。
    fn addtrks(&self, _lid: i64, _trks: &[TrackSummary]) -> Result<usize, AppError> {
        Err(crate::domain::error::AuthError::Missing.into())
    }
    /// 从指定账号歌单移除单首曲目。
    fn deltrk(&self, _lid: i64, _tid: &TrackId) -> Result<(), AppError> {
        Err(crate::domain::error::AuthError::Missing.into())
    }
    /// 删除自建歌单。
    fn dellst(&self, _lid: i64) -> Result<(), AppError> {
        Err(crate::domain::error::AuthError::Missing.into())
    }
}

/// 音频源解析端口
pub trait StreamPort: Send + Sync {
    /// 解析可播放的音频流地址
    fn rslv(&self, id: &TrackId, qlt: AudioQuality) -> Result<ResolvedSource, AppError>;
    /// 结合曲目扩展哈希解析指定音质播放地址。
    fn rslvtrk(&self, trk: &TrackSummary, qlt: AudioQuality) -> Result<ResolvedSource, AppError> {
        self.rslv(&trk.id, qlt)
    }
    /// 在线播放时按所选音质及权限向下兼容解析可用流地址。
    fn rslvply(&self, trk: &TrackSummary, qlt: AudioQuality) -> Result<ResolvedSource, AppError> {
        self.rslvtrk(trk, qlt)
    }
}

/// 播放控制端口
pub trait PlaybackPort: Send + Sync {
    /// 播放指定曲目
    fn ply(&self, src: ResolvedSource) -> Result<(), AppError>;
    /// 暂停播放
    fn ps(&self);
    /// 恢复播放
    fn rsm(&self);
    /// 停止播放
    fn stp(&self);
    /// 调整播放进度
    fn sk(&self, pos_ms: u64);
    /// 设置播放音量
    fn stvlm(&self, vlm: f32);
    /// 获取当前播放进度快照
    fn prgrss(&self) -> PlaybackProgress;
}

/// 持久化存储端口
pub trait StoragePort: Send + Sync {
    /// 读取配置
    fn ldcfg(&self) -> Result<AppSettings, AppError>;
    /// 保存配置
    fn svcfg(&self, cfg: &AppSettings) -> Result<(), AppError>;
    /// 保存播放队列
    fn svqu(&self, trks: &[TrackId], crr: Option<usize>) -> Result<(), AppError>;
    /// 读取播放队列
    fn ldqu(&self) -> Result<(Vec<TrackId>, Option<usize>), AppError>;
    /// 保存曲目元数据。
    fn svtrks(&self, tracks: &[TrackSummary]) -> Result<(), AppError>;
    /// 按标识读取曲目元数据。
    fn ldtrk(&self, id: &TrackId) -> Result<Option<TrackSummary>, AppError>;
    /// 读取本地收藏。
    fn ldfvrt(&self) -> Result<Vec<TrackSummary>, AppError>;
    /// 设置本地收藏状态。
    fn stfvrt(&self, id: &TrackId, enabled: bool) -> Result<(), AppError>;
    /// 保存用户插入的 CoverFlow 卡片。
    fn svdecks(&self, _decks: &[CoverDeck]) -> Result<(), AppError> { Ok(()) }
    /// 读取已保存的 CoverFlow 卡片。
    fn lddecks(&self) -> Result<Vec<CoverDeck>, AppError> { Ok(Vec::new()) }
    /// 记录已完成的本地下载音频索引。
    fn svdwnld(&self, _rec: &DownloadedFile) -> Result<(), AppError> { Ok(()) }
    /// 查询曲目可用的本地已下载记录（无损优先）。
    fn lddwnld(&self, _id: &TrackId) -> Result<Option<DownloadedFile>, AppError> { Ok(None) }
}
