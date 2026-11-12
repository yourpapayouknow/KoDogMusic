// Windows通知区集成。
use crate::deps::*;
use crate::deps::win::*;

static ACTION: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
static RESTART: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
const CALLBACK: u32 = WM_APP + 1;

/// 通知区图标与消息线程。
pub struct Tray { hwnd: isize, thread: Option<std::thread::JoinHandle<()>> }

impl Tray {
    /// 创建通知区图标和独立消息循环。
    pub fn nw() -> std::result::Result<Self, String> {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let thread = std::thread::spawn(move || unsafe {
            let class = wstr("KoDogLiteTray");
            let instance = GetModuleHandleW(std::ptr::null());
            let descriptor = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: instance, lpszClassName: class.as_ptr(), ..std::mem::zeroed() };
            if RegisterClassW(&descriptor) == 0 { let _ = tx.send(Err("注册通知区窗口失败".to_string())); return; }
            let hwnd = CreateWindowExW(0, class.as_ptr(), class.as_ptr(), 0, 0, 0, 0, 0, std::ptr::null_mut(), std::ptr::null_mut(), instance, std::ptr::null());
            if hwnd.is_null() { let _ = tx.send(Err("创建通知区窗口失败".to_string())); UnregisterClassW(class.as_ptr(), instance); return; }
            RESTART.store(RegisterWindowMessageW(wstr("TaskbarCreated").as_ptr()), std::sync::atomic::Ordering::Release);
            if !addicn(hwnd) {
                let _ = tx.send(Err("创建通知区图标失败".to_string())); DestroyWindow(hwnd); UnregisterClassW(class.as_ptr(), instance); return;
            }
            if tx.send(Ok(hwnd as isize)).is_err() { DestroyWindow(hwnd); }
            let mut message: MSG = std::mem::zeroed();
            while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 { TranslateMessage(&message); DispatchMessageW(&message); }
            let icon = icndata(hwnd);
            Shell_NotifyIconW(NIM_DELETE, &icon);
            UnregisterClassW(class.as_ptr(), instance);
        });
        match rx.recv().unwrap_or_else(|_| Err("通知区线程已退出".into())) {
            Ok(hwnd) => Ok(Self { hwnd, thread: Some(thread) }),
            Err(error) => { let _ = thread.join(); Err(error) }
        }
    }

    /// 取出通知区的恢复或退出请求。
    pub fn pllevt(&self) -> u8 { ACTION.swap(0, std::sync::atomic::Ordering::AcqRel) }
}

impl Drop for Tray {
    /// 删除图标并回收通知区消息线程。
    fn drop(&mut self) {
        unsafe { PostMessageW(self.hwnd as HWND, WM_CLOSE, 0, 0); }
        if let Some(thread) = self.thread.take() { let _ = thread.join(); }
    }
}

/// 生成Windows空字符结尾文本。
fn wstr(text: &str) -> Vec<u16> { text.encode_utf16().chain(Some(0)).collect() }

/// 创建通知区图标数据。
unsafe fn icndata(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = hwnd;
    data.uID = 1;
    data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    data.uCallbackMessage = CALLBACK;
    data.hIcon = unsafe { LoadIconW(std::ptr::null_mut(), IDI_APPLICATION) };
    let tip = wstr("KoDog Lite");
    data.szTip[..tip.len()].copy_from_slice(&tip);
    data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
    data
}

/// 添加图标并注册通知协议版本。
unsafe fn addicn(hwnd: HWND) -> bool {
    let data = unsafe { icndata(hwnd) };
    if unsafe { Shell_NotifyIconW(NIM_ADD, &data) } == 0 { return false; }
    unsafe { Shell_NotifyIconW(NIM_SETVERSION, &data); }
    true
}

/// 处理通知区菜单和Windows任务栏重启。
unsafe extern "system" fn wndproc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        let restart = RESTART.load(std::sync::atomic::Ordering::Acquire);
        if restart != 0 && message == restart { addicn(hwnd); return 0; }
        match message {
            CALLBACK => {
                let event = (lparam as u32) & 0xFFFF;
                if event == NIN_SELECT || event == WM_LBUTTONUP || event == WM_LBUTTONDBLCLK {
                    ACTION.store(1, std::sync::atomic::Ordering::Release);
                } else if event == WM_CONTEXTMENU || event == WM_RBUTTONUP {
                    let menu = CreatePopupMenu();
                    if !menu.is_null() {
                        AppendMenuW(menu, MF_STRING, 1, wstr("恢复窗口").as_ptr());
                        AppendMenuW(menu, MF_STRING, 2, wstr("退出").as_ptr());
                        let mut point: POINT = std::mem::zeroed();
                        GetCursorPos(&mut point);
                        SetForegroundWindow(hwnd);
                        let selected = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON, point.x, point.y, 0, hwnd, std::ptr::null());
                        DestroyMenu(menu);
                        PostMessageW(hwnd, WM_NULL, 0, 0);
                        if selected == 1 || selected == 2 { ACTION.store(selected as u8, std::sync::atomic::Ordering::Release); }
                    }
                }
                0
            }
            WM_CLOSE => { DestroyWindow(hwnd); 0 }
            WM_DESTROY => { PostQuitMessage(0); 0 }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }
}

/// 取窗口的 Win32 句柄。
fn winhwnd(window: &Window) -> Option<HWND> {
    let raw = window.window_handle();
    let handle = HasWindowHandle::window_handle(&raw).ok()?;
    match handle.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get() as HWND),
        _ => None,
    }
}

/// 隐藏到通知区或恢复原生主窗口。
pub fn vsbl(window: &Window, visible: bool) {
    if visible { let _ = window.show(); window.set_minimized(false); } else { let _ = window.hide(); }
    if visible && let Some(hwnd) = winhwnd(window) { unsafe { SetForegroundWindow(hwnd); } }
}

/// 将桌面歌词置于普通应用窗口上方并启用整窗拖动。
pub fn pnwin(window: &Window) -> bool {
    let applied = winhwnd(window).is_some_and(|hwnd| unsafe {
        SetWindowLongW(hwnd, GWL_STYLE, GetWindowLongW(hwnd, GWL_STYLE) | WS_SYSMENU as i32);
        SetWindowLongW(hwnd, GWL_EXSTYLE, (GetWindowLongW(hwnd, GWL_EXSTYLE) | WS_EX_TOOLWINDOW as i32) & !(WS_EX_APPWINDOW as i32));
        SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED);
        // 复用主窗圆角方案。
        rndwnd(hwnd);
        true
    });
    applied && drgwin(window, true).is_ok()
}

/// 销毁桌面歌词原生窗口。
pub fn rmlwin(window: &Window) {
    if let Some(hwnd) = winhwnd(window) { unsafe { DestroyWindow(hwnd); } }
}

/// 为主窗顶边和歌词窗复用系统的非客户区移动处理。
pub fn drgwin(window: &Window, full: bool) -> std::result::Result<(), String> {
    let Some(hwnd) = winhwnd(window) else { return Err("窗口句柄不可用".into()); };
    let mode = if full { 0 } else { 1 };
    if unsafe { SetWindowSubclass(hwnd, Some(drgproc), 1, mode) } == 0 {
        return Err(format!("窗口子类安装失败：{}", unsafe { GetLastError() }));
    }
    if !full { unsafe { rndwnd(hwnd); } }
    Ok(())
}

/// 跨屏缩放变化后同步圆角样式与整窗重绘。
pub fn synwnd(window: &Window) {
    if let Some(hwnd) = winhwnd(window) {
        unsafe {
            if IsIconic(hwnd) == 0 { rndwnd(hwnd); }
            InvalidateRect(hwnd, std::ptr::null(), 0);
        }
    }
}

/// 同步系统窗口明暗边框模式。
pub fn stthm(window: &Window, dark: bool) {
    if let Some(hwnd) = winhwnd(window) {
        let value: i32 = if dark { 1 } else { 0 };
        unsafe {
            DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE as u32, (&raw const value).cast(), std::mem::size_of::<i32>() as u32);
        }
    }
}

/// 在左上大卡片等可交互区开始拖拽后将移动控制权交回 Windows。
pub fn drgwnd(window: &Window) {
    if let Some(hwnd) = winhwnd(window) {
        unsafe {
            if (GetAsyncKeyState(VK_LBUTTON as i32) as u16 & 0x8000) == 0 { return; }
            let mut pt: POINT = std::mem::zeroed();
            GetCursorPos(&mut pt);
            let lparam = (((pt.y as i16 as u16) as usize) << 16 | ((pt.x as i16 as u16) as usize)) as isize;
            ReleaseCapture();
            PostMessageW(hwnd, WM_NCLBUTTONDOWN, HTCAPTION as usize, lparam);
        }
    }
}

/// 将标题区域命中交给 Windows，并在尺寸或 DPI 变化后保持圆角与重绘。
unsafe extern "system" fn drgproc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM, identity: usize, mode: usize) -> LRESULT {
    unsafe {
        if message == WM_NCHITTEST {
            if mode == 0 { return HTCAPTION as LRESULT; }
            let mut point = POINT { x: (lparam as u32 as u16 as i16) as i32, y: ((lparam as u32 >> 16) as u16 as i16) as i32 };
            let mut rect: RECT = std::mem::zeroed();
            if GetWindowRect(hwnd, &mut rect) != 0 && ScreenToClient(hwnd, &mut point) != 0 {
                let width = (rect.right - rect.left).max(1);
                let height = (rect.bottom - rect.top).max(1);
                let lx = point.x * 360 / width;
                let ly = point.y * 480 / height;
                if ly >= 0 && (ly < 6 || (ly < 14 && lx >= 312) || (ly < 42 && (306..312).contains(&lx))) {
                    return HTCAPTION as LRESULT;
                }
            }
        }
        if message == WM_NCLBUTTONDOWN && wparam == HTCAPTION as usize {
            let res = DefWindowProcW(hwnd, message, wparam, lparam);
            if mode != 0 && (GetAsyncKeyState(VK_LBUTTON as i32) as u16 & 0x8000) == 0 {
                let mut pt: POINT = std::mem::zeroed();
                if GetCursorPos(&mut pt) != 0 && ScreenToClient(hwnd, &mut pt) != 0 {
                    let up = (((pt.y as i16 as u16) as usize) << 16 | ((pt.x as i16 as u16) as usize)) as isize;
                    PostMessageW(hwnd, WM_LBUTTONUP, 0, up);
                }
            }
            return res;
        }
        if message == WM_NCDESTROY { RemoveWindowSubclass(hwnd, Some(drgproc), identity); }
        let result = DefSubclassProc(hwnd, message, wparam, lparam);
        if mode != 0 && IsIconic(hwnd) == 0 && ((message == WM_SIZE && wparam != SIZE_MINIMIZED as usize) || message == WM_WINDOWPOSCHANGED || message == WM_DPICHANGED) {
            if !dwmrnd(hwnd) { rgnrnd(hwnd); }
        }
        if message == WM_DPICHANGED { InvalidateRect(hwnd, std::ptr::null(), 0); }
        result
    }
}

/// 优先启用 Windows 11 原生 DWM 抗锯齿圆角与阴影，不支持时回退区域裁切。
unsafe fn rndwnd(hwnd: HWND) {
    if unsafe { dwmrnd(hwnd) } { return; }
    unsafe { rgnrnd(hwnd); }
}

/// 启用系统级圆角与阴影扩展，并清除旧版 GDI 裁切区域。
unsafe fn dwmrnd(hwnd: HWND) -> bool {
    let pref = DWMWCP_ROUND;
    let ok = unsafe { DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE as u32, (&raw const pref).cast(), std::mem::size_of::<i32>() as u32) } == 0;
    if !ok { return false; }
    let margins = MARGINS { cxLeftWidth: 1, cxRightWidth: 1, cyTopHeight: 1, cyBottomHeight: 1 };
    unsafe { DwmExtendFrameIntoClientArea(hwnd, &margins); }
    let existing = unsafe { CreateRectRgn(0, 0, 0, 0) };
    if !existing.is_null() {
        if unsafe { GetWindowRgn(hwnd, existing) } != 0 { unsafe { SetWindowRgn(hwnd, std::ptr::null_mut(), 1); } }
        unsafe { DeleteObject(existing); }
    }
    true
}

/// 在不支持 DWM 圆角的系统上使用窗口区域裁出八像素圆角。
unsafe fn rgnrnd(hwnd: HWND) {
    let mut rect: RECT = unsafe { std::mem::zeroed() };
    if unsafe { GetWindowRect(hwnd, &mut rect) } == 0 { return; }
    let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
    if width <= 0 || height <= 0 { return; }
    let diameter = (16 * width / 360).max(1);
    let region = unsafe { CreateRoundRectRgn(0, 0, width + 1, height + 1, diameter, diameter) };
    if region.is_null() { return; }
    let existing = unsafe { CreateRectRgn(0, 0, 0, 0) };
    if !existing.is_null() {
        let same = unsafe { GetWindowRgn(hwnd, existing) } != 0 && unsafe { EqualRgn(existing, region) } != 0;
        unsafe { DeleteObject(existing); }
        if same { unsafe { DeleteObject(region); } return; }
    }
    if unsafe { SetWindowRgn(hwnd, region, 1) } == 0 { unsafe { DeleteObject(region); } }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 真实窗口应优先获得 DWM 原生圆角或区域圆角、支持跨屏缩放并保持顶部拖动热区。
    #[test]
    fn tst_wndround() {
        unsafe {
            let class = wstr("KoDogRoundTest");
            let instance = GetModuleHandleW(std::ptr::null());
            let descriptor = WNDCLASSW { lpfnWndProc: Some(DefWindowProcW), hInstance: instance, lpszClassName: class.as_ptr(), ..std::mem::zeroed() };
            assert_ne!(RegisterClassW(&descriptor), 0);
            let hwnd = CreateWindowExW(WS_EX_TOOLWINDOW, class.as_ptr(), class.as_ptr(), WS_POPUP, 0, 0, 360, 480, std::ptr::null_mut(), std::ptr::null_mut(), instance, std::ptr::null());
            assert!(!hwnd.is_null());
            let region = CreateRectRgn(0, 0, 0, 0);
            rgnrnd(hwnd);
            let fallback_ok = GetWindowRgn(hwnd, region) != 0 && PtInRegion(region, 0, 0) == 0 && PtInRegion(region, 180, 240) != 0;
            let installed = SetWindowSubclass(hwnd, Some(drgproc), 1, 1);
            rndwnd(hwnd);
            if dwmrnd(hwnd) {
                assert_eq!(GetWindowRgn(hwnd, region), 0, "DWM rounded window must not retain legacy GDI region");
            } else {
                SetWindowPos(hwnd, std::ptr::null_mut(), 0, 0, 450, 600, SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOZORDER);
                assert!(GetWindowRgn(hwnd, region) != 0 && PtInRegion(region, 449, 599) == 0);
            }
            SetWindowPos(hwnd, std::ptr::null_mut(), 0, 0, 450, 600, SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOZORDER);
            let hit_top = SendMessageW(hwnd, WM_NCHITTEST, 0, ((4i32 << 16) | 180i32) as LPARAM);
            let hit_min_top = SendMessageW(hwnd, WM_NCHITTEST, 0, ((10i32 << 16) | 410i32) as LPARAM);
            let hit_gap = SendMessageW(hwnd, WM_NCHITTEST, 0, ((25i32 << 16) | 386i32) as LPARAM);
            let hit_body = SendMessageW(hwnd, WM_NCHITTEST, 0, ((60i32 << 16) | 180i32) as LPARAM);
            DeleteObject(region); DestroyWindow(hwnd); UnregisterClassW(class.as_ptr(), instance);
            assert_ne!(installed, 0);
            assert!(fallback_ok, "region fallback rounded corners failed");
            assert_eq!(hit_top, HTCAPTION as LRESULT, "top drag strip did not return HTCAPTION");
            assert_eq!(hit_min_top, HTCAPTION as LRESULT, "header above window controls did not return HTCAPTION");
            assert_eq!(hit_gap, HTCAPTION as LRESULT, "header gap before minimize did not return HTCAPTION");
            assert_ne!(hit_body, HTCAPTION as LRESULT, "body should not trigger caption drag");
        }
    }

    /// 验证真实通知区消息可触发恢复并回收图标线程。
    #[test]
    fn tst_tray() {
        let tray = Tray::nw().unwrap();
        unsafe { PostMessageW(tray.hwnd as HWND, CALLBACK, 0, NIN_SELECT as LPARAM); }
        let mut action = 0;
        for _ in 0..200 {
            action = tray.pllevt();
            if action != 0 { break; }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(action, 1);
    }
}
