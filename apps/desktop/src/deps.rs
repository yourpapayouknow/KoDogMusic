// 桌面端外部依赖统一入口。
#![allow(unused_imports)]
// Slint界面与软件渲染。
pub use slint::{ComponentHandle, Image, ModelRc, VecModel, SharedString, SharedPixelBuffer, Rgba8Pixel, Timer, TimerMode, Window, LogicalSize, CloseRequestResponse, Weak, BackendSelector, quit_event_loop, run_event_loop_until_quit};
pub use slint::{PhysicalSize, PlatformError, Rgb8Pixel};
pub mod plat {
    pub use slint::platform::{set_platform, update_timers_and_animations, Platform, WindowAdapter};
    pub use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
}
pub type Frame = SharedPixelBuffer<Rgba8Pixel>;
// 图像、二维码与系统API。
pub use image;
pub use image::{Rgba, RgbaImage};
pub use unicode_segmentation::UnicodeSegmentation;
pub use raw_window_handle::{HasWindowHandle, RawWindowHandle};
pub use qrcode::{QrCode, Color as QrColor};
pub mod win {
    pub use windows_sys::Win32::{Foundation::*, Graphics::{Dwm::{DwmSetWindowAttribute, DwmExtendFrameIntoClientArea, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DWMWA_USE_IMMERSIVE_DARK_MODE}, Gdi::{ScreenToClient, CreateRoundRectRgn, CreateRectRgn, GetWindowRgn, EqualRgn, PtInRegion, SetWindowRgn, InvalidateRect, DeleteObject, MonitorFromPoint, GetMonitorInfoW, MONITORINFO, MONITOR_DEFAULTTONEAREST}}, UI::{Controls::MARGINS, Input::KeyboardAndMouse::{ReleaseCapture, GetAsyncKeyState, VK_LBUTTON}, WindowsAndMessaging::*, Shell::*}, System::LibraryLoader::GetModuleHandleW};
}
pub use std::{borrow::Cow, ops::Range, path::{Path, PathBuf}, sync::{Arc, Mutex}, time::Duration};
pub use std::{rc::Rc, cell::RefCell, time::Instant};
// 核心业务与数据模型。
pub use kugou_core::application::{commands::{AppCommand, AppRoute, TargetScope}, coordinator::Coordinator, snapshot::{AddToPrompt, DeckSummary, DownloadPrompt, OpenedPlaylist, UiSnapshot}};
pub use kugou_core::domain::{account::{AccountState, CheckInState, QrStage}, ids::{ProviderId, TrackId}, playback::{PlaybackState, RepeatMode}, settings::{CacheSettings, CloseAction, ThemeMode}, track::{AudioQuality, DeckKind, LyricLine, TrackSummary, UserPlaylistMeta}, error::AppError as CoreError};
pub use kugou_core::services::{downloads::DownloadState, images::ldimage};
pub use kugou_core::deps::{md5::{Digest, Md5}, uuid::Uuid};
pub use kugou_core::audio::controller::AudioController;
pub use kugou_core::provider::kugou::{auth::KugouAuth, catalog::KugouCatalog, client::KugouClient, library::KugouLibrary, stream::KugouStream};
pub use kugou_core::storage::database::DatabaseStorage;
