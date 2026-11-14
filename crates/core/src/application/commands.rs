use crate::deps::serde::{Deserialize, Serialize};
use crate::domain::ids::TrackId;
use crate::domain::playback::RepeatMode;
use crate::domain::settings::{CloseAction, ThemeMode};
use crate::domain::track::AudioQuality;

/// 页面路由枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AppRoute {
    #[default]
    Player,
    Queue,
    Search,
    Favorites,
    Settings,
}

/// 添加或下载操作的目标范围。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TargetScope {
    Single,
    Collection,
}

/// 客户端全局应用命令
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AppCommand {
    /// 导航至指定页面
    Nav(AppRoute),
    /// 请求登录
    AuthLogin,
    /// 查询当前二维码授权。
    AuthPoll,
    /// 关闭二维码页面。
    AuthCancel,
    /// 请求登出
    AuthLogout,
    /// 每日签到
    CheckIn,
    /// 加载并组装登录后的 CoverFlow 播放列表集合。
    LoadCoverFlow,
    /// 切换当前 CoverFlow 封面集合并按需起播。
    SelectDeck { index: usize, auto_play: bool },
    /// 剔除用户主动加入 CoverFlow 的自定义封面。
    RemoveDeck(usize),
    /// 搜索单曲
    Search(String),
    /// 打开「添加至」二级弹窗（单曲或整张专辑）。
    OpenAddTo { search_index: usize, whole_album: bool },
    /// 关闭「添加至」二级弹窗。
    CancelAddTo,
    /// 创建用户云端歌单。
    CreatePlaylist(String),
    /// 确认提交「添加至」勾选目标（CoverFlow / Like / 歌单列表）。
    ConfirmAddTo { to_coverflow: bool, to_like: bool, playlist_ids: Vec<String> },
    /// 打开收藏页中的指定歌单详情。
    OpenPlaylist(String),
    /// 返回收藏页歌单列表。
    ClosePlaylist,
    /// 在收藏页中播放或将歌单加入 CoverFlow。
    PlayPlaylist(String),
    /// 将收藏页歌单插入到 CoverFlow 日推右侧。
    AddPlaylistToFlow(String),
    /// 播放指定曲目
    PlayTrack(TrackId),
    /// 播放指定队列位置，保留重复曲目的独立位置。
    QueuePlay(usize),
    /// 点击底部爱心：未喜欢直接收藏，已喜欢触发二次确认弹窗。
    HeartClick,
    /// 处理取消喜欢确认弹窗响应。
    ConfirmUnlike(bool),
    /// 按当前音质下载指定曲目。
    Download(TrackId),
    /// 探测当前曲目或集合的上游可用下载品质并弹出品质选择弹窗。
    InspectDownload { scope: TargetScope, search_index: Option<usize>, whole_album: bool },
    /// 确认按所选品质批量/单曲下载。
    ConfirmDownload(AudioQuality),
    /// 取消下载品质选择弹窗。
    CancelDownloadPrompt,
    /// 加入播放队列。
    QueueAdd(TrackId),
    /// 移除指定队列项。
    QueueRemove(usize),
    /// 清空播放队列。
    QueueClear,
    /// 切换播放/暂停
    TogglePlay,
    /// 下一首
    Next,
    /// 上一首
    Prev,
    /// 停止播放
    Stop,
    /// 进度跳转
    Seek(u64),
    /// 设置音量
    SetVolume(f32),
    /// 设置循环模式
    SetRepeat(RepeatMode),
    /// 切换随机播放
    ToggleShuffle,
    /// 收藏或取消收藏
    ToggleFavorite(TrackId),
    /// 将搜索曲目加入指定歌单并立即播放（prepend 时置于歌单首）
    AddPlaylistJumpPlay { search_index: usize, playlist_id: String, prepend: bool },
    /// 打开搜索曲目的专辑视图
    OpenAlbum(usize),
    /// 关闭专辑视图
    CloseAlbum,
    /// 下载单曲歌词文件
    DownloadLyrics(TrackId),
    /// 从打开的账号歌单移除指定曲目
    PlaylistRemoveTrack(usize),
    /// 载入打开的歌单为队列并从指定曲目播放
    PlaylistPlay { index: usize },
    /// 载入专辑视图对应的专辑为队列并从指定曲目播放
    AlbumPlay { index: usize },
    /// 删除自建歌单
    DeletePlaylist(String),
    /// 按专辑标识打开专辑视图
    OpenAlbumById(String),
    /// 刷新账号收藏并明确本地回退来源。
    RefreshFavorites,
    /// 切换主题模式
    SetTheme(ThemeMode),
    /// 设置期望音质
    SetQuality(AudioQuality),
    /// 设置关闭时的行为。
    SetClose(CloseAction),
    /// 切换桌面歌词。
    SetLyrics(bool),
    /// 设置缓存目录。
    SetCacheDir(std::path::PathBuf),
    /// 设置下载目录。
    SetDownloadDir(std::path::PathBuf),
}
