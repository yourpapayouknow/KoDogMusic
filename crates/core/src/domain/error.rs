use crate::deps::thiserror::Error;

/// 鉴权类错误
#[derive(Debug, Error)]
pub enum AuthError {
    #[error("会话已过期")]
    Expired,
    #[error("访问被拒绝")]
    Denied,
    #[error("缺少登录凭据")]
    Missing,
}

/// 网络通信类错误
#[derive(Debug, Error)]
pub enum NetworkError {
    #[error("网络请求超时")]
    Timeout,
    #[error("网络离线或无法连接")]
    Offline,
    #[error("触发限流保护")]
    RateLimited,
    #[error("服务端异常: {0}")]
    Server(String),
}

/// 媒体目录与元数据类错误
#[derive(Debug, Error)]
pub enum CatalogError {
    #[error("未找到对应资源")]
    NotFound,
    #[error("元数据格式非法: {0}")]
    InvalidData(String),
}

/// 播放与解码类错误
#[derive(Debug, Error)]
pub enum PlaybackError {
    #[error("播放源授权已过期")]
    SourceExpired,
    #[error("解码失败: {0}")]
    Decode(String),
    #[error("音频输出设备异常: {0}")]
    Device(String),
    #[error("音频缓冲区欠载")]
    Underrun,
    #[error("不支持的音频格式")]
    UnsupportedFormat,
    #[error("该曲目未提供所选音质")]
    Quality,
}

/// 存储与本地数据库类错误
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("数据库或数据文件损坏")]
    Corrupt,
    #[error("权限不足")]
    Permission,
    #[error("磁盘空间已满")]
    DiskFull,
    #[error("数据库迁移失败: {0}")]
    Migration(String),
    #[error("文件读写失败: {0}")]
    Io(String),
}

/// 客户端全局统一错误
#[derive(Debug, Error)]
pub enum AppError {
    #[error(transparent)]
    Auth(#[from] AuthError),
    #[error(transparent)]
    Network(#[from] NetworkError),
    #[error(transparent)]
    Catalog(#[from] CatalogError),
    #[error(transparent)]
    Playback(#[from] PlaybackError),
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("操作已取消")]
    Cancelled,
}
