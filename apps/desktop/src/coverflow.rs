use crate::deps::{Arc, Frame, Instant, Mutex, Rgba, RgbaImage};
use std::sync::Condvar;

const FRICTION: f32 = 0.72;
const DAMPING: f32 = 0.16;

/// 早期 Apple iTunes 3D CoverFlow 原生软件渲染器
pub struct CoverFlowEngine {
    slots: Vec<(usize, Option<Arc<RgbaImage>>, Option<[u8; 3]>)>,
    qr_cover: Option<Arc<RgbaImage>>,
    total: usize,
    anchor: usize,
    current_offset: f32,
    target_offset: f32,
    vel: f32,
    drg: bool,
    trvl: f32,
    scratch: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub background: [u8; 3],
}

/// 异步渲染线程与界面共享的轻量状态。
struct FlwShrd {
    engine: CoverFlowEngine,
    visible: bool,
    dirty: bool,
    stop: bool,
}

/// 独立后台线程异步光栅化 CoverFlow 帧，不阻塞界面拖拽与滚动。
pub struct FlwWrkr {
    shared: Arc<(Mutex<FlwShrd>, Condvar)>,
    latest: Arc<Mutex<Option<Arc<Frame>>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl FlwWrkr {
    /// 启动空闲零占用的独立 CoverFlow 渲染线程。
    pub fn nw(width: u32, height: u32, background: [u8; 3]) -> Self {
        let mut engine = CoverFlowEngine::new(width, height);
        engine.background = background;
        let initial = engine.render_frame();
        let shared = Arc::new((Mutex::new(FlwShrd { engine, visible: true, dirty: false, stop: false }), Condvar::new()));
        let latest = Arc::new(Mutex::new(Some(initial)));
        let st = shared.clone();
        let out = latest.clone();
        let thread = std::thread::spawn(move || {
            let mut scratch = Vec::<u8>::new();
            let mut last = Instant::now();
            loop {
                let (slots, qr, cur, w, h, bg, active) = {
                    let mut guard = st.0.lock().unwrap();
                    loop {
                        if guard.stop { return; }
                        let can_run = guard.visible && (guard.dirty || guard.engine.active());
                        if can_run { break; }
                        guard = st.1.wait(guard).unwrap();
                        last = Instant::now();
                    }
                    let now = Instant::now();
                    let dt_ms = ((now - last).as_secs_f32() * 1000.0).clamp(1.0, 50.0);
                    last = now;
                    guard.engine.flwstep(dt_ms);
                    guard.dirty = false;
                    let active = guard.engine.active();
                    (
                        guard.engine.slots.clone(),
                        guard.engine.qr_cover.clone(),
                        guard.engine.current_offset,
                        guard.engine.width,
                        guard.engine.height,
                        guard.engine.background,
                        active,
                    )
                };
                let frame = rndrscn(&mut scratch, &slots, qr.as_deref(), cur, w, h, bg);
                *out.lock().unwrap() = Some(frame);
                if active {
                    let elapsed = last.elapsed();
                    if elapsed < std::time::Duration::from_millis(14) {
                        std::thread::sleep(std::time::Duration::from_millis(14) - elapsed);
                    }
                }
            }
        });
        Self { shared, latest, thread: Some(thread) }
    }

    /// 读取最新异步渲染帧指针。
    pub fn pllfrm(&self) -> Option<Arc<Frame>> {
        self.latest.lock().unwrap().clone()
    }

    /// 返回当前是否正在运行拖拽或吸附动画。
    pub fn active(&self) -> bool {
        self.shared.0.lock().unwrap().engine.active()
    }

    /// 返回当前最接近中心的目标索引。
    pub fn curidx(&self) -> usize {
        self.shared.0.lock().unwrap().engine.selidx()
    }

    /// 设置首页二维码卡片像素并重绘。
    pub fn stqr(&self, qr: Option<Arc<RgbaImage>>) {
        let mut guard = self.shared.0.lock().unwrap();
        let changed = match (&guard.engine.qr_cover, &qr) {
            (Some(a), Some(b)) => !Arc::ptr_eq(a, b),
            (None, None) => false,
            _ => true,
        };
        if changed {
            guard.engine.qr_cover = qr.map(|img| Arc::new(mkqrcrd(&img)));
            guard.dirty = true;
            self.shared.1.notify_one();
        }
    }

    /// 设置画布可见性以在隐藏或切页时暂停渲染。
    pub fn stvis(&self, visible: bool) {
        let mut guard = self.shared.0.lock().unwrap();
        if guard.visible != visible {
            guard.visible = visible;
            if visible { guard.dirty = true; }
            self.shared.1.notify_one();
        }
    }

    /// 同步 DPI 缩放后的画布物理尺寸。
    pub fn rsz(&self, width: u32, height: u32) {
        let mut guard = self.shared.0.lock().unwrap();
        if guard.engine.width != width || guard.engine.height != height {
            guard.engine.rsz(width, height);
            guard.dirty = true;
            self.shared.1.notify_one();
        }
    }

    /// 同步主题背景色并触发重绘。
    pub fn stbg(&self, background: [u8; 3]) {
        let mut guard = self.shared.0.lock().unwrap();
        if guard.engine.background != background {
            guard.engine.background = background;
            guard.dirty = true;
            self.shared.1.notify_one();
        }
    }

    /// 更新滑动窗口封面并唤醒渲染线程。
    pub fn stwin(&self, slots: Vec<(usize, Option<Arc<RgbaImage>>, Option<[u8; 3]>)>, total: usize, target: usize) {
        let mut guard = self.shared.0.lock().unwrap();
        guard.engine.stwin(slots, total, target);
        guard.dirty = true;
        self.shared.1.notify_one();
    }

    /// 登录后启动非线性自动往后 flow 动画并精准停靠到日推卡片。
    pub fn flwlogin(&self, slots: Vec<(usize, Option<Arc<RgbaImage>>, Option<[u8; 3]>)>, total: usize, target: usize) {
        let mut guard = self.shared.0.lock().unwrap();
        guard.engine.stwin(slots, total, target);
        guard.engine.flwlogin(target);
        guard.dirty = true;
        self.shared.1.notify_one();
    }

    /// 开始水平拖拽。
    pub fn drgstrt(&self) {
        let mut guard = self.shared.0.lock().unwrap();
        guard.engine.drgstrt();
        guard.dirty = true;
        self.shared.1.notify_one();
    }

    /// 更新指针水平位移并唤醒异步重绘。
    pub fn drgmv(&self, dx: f32) {
        let mut guard = self.shared.0.lock().unwrap();
        guard.engine.drgmv(dx);
        guard.dirty = true;
        self.shared.1.notify_one();
    }

    /// 结束拖拽并同步返回惯性吸附目标索引。
    pub fn drgrel(&self, min_travel_px: f32) -> Option<usize> {
        let mut guard = self.shared.0.lock().unwrap();
        let res = guard.engine.drgrel(min_travel_px);
        guard.dirty = true;
        self.shared.1.notify_one();
        res
    }
}

impl Drop for FlwWrkr {
    /// 通知并回收异步渲染线程。
    fn drop(&mut self) {
        {
            let mut guard = self.shared.0.lock().unwrap();
            guard.stop = true;
            self.shared.1.notify_all();
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl CoverFlowEngine {
    /// 创建封面投影画布。
    pub fn new(width: u32, height: u32) -> Self {
        let size = (width as usize).saturating_mul(height as usize).saturating_mul(4);
        Self {
            slots: Vec::new(),
            qr_cover: None,
            total: 0,
            anchor: 0,
            current_offset: 0.0,
            target_offset: 0.0,
            vel: 0.0,
            drg: false,
            trvl: 0.0,
            scratch: vec![0; size],
            width,
            height,
            background: [241, 235, 229],
        }
    }

    /// 设置登录后的非线性自动流动起点与停靠目标。
    pub fn flwlogin(&mut self, target: usize) {
        if self.total == 0 { return; }
        let clamped = target.min(self.total - 1);
        self.anchor = clamped;
        self.current_offset = -1.6;
        self.target_offset = clamped as f32;
        self.vel = 0.10;
        self.drg = false;
    }

    /// 调整画布与复用像素缓冲尺寸。
    pub fn rsz(&mut self, width: u32, height: u32) {
        if self.width == width && self.height == height {
            return;
        }
        self.width = width;
        self.height = height;
        let size = (width as usize).saturating_mul(height as usize).saturating_mul(4);
        self.scratch.resize(size, 0);
    }

    /// 更新滑动队列封面窗口并保留连续动画轨迹。
    pub fn stwin(&mut self, slots: Vec<(usize, Option<Arc<RgbaImage>>, Option<[u8; 3]>)>, total: usize, target: usize) {
        let was_empty = self.total == 0;
        self.slots = slots;
        self.total = total;
        if total == 0 {
            self.anchor = 0;
            self.current_offset = 0.0;
            self.target_offset = 0.0;
            self.vel = 0.0;
            self.drg = false;
            return;
        }
        let clamped = target.min(total - 1);
        self.anchor = clamped;
        if was_empty {
            self.current_offset = clamped as f32;
            self.target_offset = clamped as f32;
            self.vel = 0.0;
        } else if !self.drg {
            self.target_offset = clamped as f32;
            let max = (total - 1) as f32;
            if self.current_offset > max + 1.0 {
                self.current_offset = max;
            }
        }
    }

    /// 添加一张唱片封面。
    #[cfg(test)]
    pub fn add_cover(&mut self, img: RgbaImage) {
        let idx = self.slots.len();
        self.slots.push((idx, Some(Arc::new(img)), None));
        self.total = self.slots.len();
        if self.total == 1 {
            self.current_offset = 0.0;
            self.target_offset = 0.0;
            self.anchor = 0;
        }
    }

    /// 设置目标索引。
    #[cfg(test)]
    pub fn sttgt(&mut self, index: usize) {
        if self.total > 0 {
            let clamped = index.min(self.total - 1);
            self.anchor = clamped;
            self.target_offset = clamped as f32;
        }
    }

    /// 移动相对偏移。
    #[cfg(test)]
    pub fn scroll_delta(&mut self, delta: f32) {
        if self.total > 0 {
            let max_idx = (self.total - 1) as f32;
            self.target_offset = (self.target_offset + delta).clamp(0.0, max_idx);
            self.anchor = self.target_offset.round() as usize;
        }
    }

    /// 获取当前最接近的索引。
    pub fn selidx(&self) -> usize {
        if self.total == 0 {
            return 0;
        }
        self.target_offset.round().clamp(0.0, (self.total - 1) as f32) as usize
    }

    /// 获取每次拖拽切换一张封面的物理像素步长。
    fn drgpx(&self) -> f32 {
        (96.0 * (self.width as f32 / 360.0)).max(24.0)
    }

    /// 开始水平拖拽 CoverFlow。
    pub fn drgstrt(&mut self) {
        if self.total == 0 {
            return;
        }
        self.drg = true;
        self.vel = 0.0;
        self.trvl = 0.0;
        self.target_offset = self.current_offset;
    }

    /// 按指针水平位移更新拖拽目标与边缘弹性回拉。
    pub fn drgmv(&mut self, dx: f32) {
        if !self.drg || self.total == 0 {
            return;
        }
        self.trvl += dx.abs();
        let step = -dx / self.drgpx();
        let max = (self.total - 1) as f32;
        let next = self.target_offset + step;
        if next < 0.0 || next > max {
            self.target_offset += step * 0.2;
        } else {
            self.target_offset = next;
        }
        self.vel = self.vel * 0.4 + step * 0.6;
    }

    /// 结束拖拽并按 Blossom 投射公式吸附到最近曲目。
    pub fn drgrel(&mut self, min_travel_px: f32) -> Option<usize> {
        if !self.drg {
            return None;
        }
        self.drg = false;
        if self.total == 0 {
            return None;
        }
        if self.trvl <= min_travel_px {
            self.vel = 0.0;
            self.target_offset = self.anchor as f32;
            return None;
        }
        let max = (self.total - 1) as f32;
        let projected = self.target_offset + (self.vel * 2.2).clamp(-2.8, 2.8) / (1.0 - FRICTION);
        let snap = projected.round().clamp(0.0, max) as usize;
        self.anchor = snap;
        self.target_offset = snap as f32;
        self.vel = (self.target_offset - self.current_offset) * (1.0 - FRICTION) * 0.25;
        Some(snap)
    }

    /// 返回当前是否仍有惯性或吸附动画在运行。
    pub fn active(&self) -> bool {
        self.drg || (self.target_offset - self.current_offset).abs() > 0.0015 || self.vel.abs() > 0.0015
    }

    /// 按帧时间间隔推进帧率无关的指数阻尼动画。
    pub fn flwstep(&mut self, dt_ms: f32) -> bool {
        let dt = (dt_ms.clamp(1.0, 50.0)) / (1000.0 / 60.0);
        if self.drg {
            let factor = 1.0 - (1.0 - FRICTION).powf(dt);
            let before = self.current_offset;
            self.current_offset += (self.target_offset - self.current_offset) * factor;
            self.vel *= FRICTION.powf(dt);
            return (self.current_offset - before).abs() > 0.0005 || (self.target_offset - self.current_offset).abs() > 0.0005;
        }
        let diff = self.target_offset - self.current_offset;
        if diff.abs() <= 0.0015 && self.vel.abs() <= 0.0015 {
            if self.current_offset != self.target_offset || self.vel != 0.0 {
                self.current_offset = self.target_offset;
                self.vel = 0.0;
                return true;
            }
            return false;
        }
        let factor = 1.0 - (1.0 - DAMPING).powf(dt);
        self.vel *= FRICTION.powf(dt);
        self.current_offset += diff * factor + self.vel * 0.35;
        if (self.target_offset - self.current_offset).abs() <= 0.0015 && self.vel.abs() <= 0.0015 {
            self.current_offset = self.target_offset;
            self.vel = 0.0;
        }
        true
    }

    /// 每帧更新动画阻尼插值（固定 60Hz 步长，供单测复用）。
    #[cfg(test)]
    pub fn tick(&mut self) -> bool {
        self.flwstep(1000.0 / 60.0)
    }

    /// 直接绘入共享RGBA缓冲。
    pub fn render_frame(&mut self) -> Arc<Frame> {
        rndrscn(&mut self.scratch, &self.slots, self.qr_cover.as_deref(), self.current_offset, self.width, self.height, self.background)
    }
}

/// 将给定位置与封面槽位绘制到可复用像素缓冲并返回共享帧。
fn rndrscn(scratch: &mut Vec<u8>, slots: &[(usize, Option<Arc<RgbaImage>>, Option<[u8; 3]>)], qr: Option<&RgbaImage>, cur: f32, w: u32, h: u32, bg: [u8; 3]) -> Arc<Frame> {
    let expected = (w as usize).saturating_mul(h as usize).saturating_mul(4);
    if scratch.len() != expected {
        scratch.resize(expected, 0);
    }

    for chunk in scratch.chunks_exact_mut(4) {
        chunk[0] = bg[0];
        chunk[1] = bg[1];
        chunk[2] = bg[2];
        chunk[3] = 255;
    }

    if !slots.iter().any(|(_, cover, _)| cover.is_some()) {
        if let Some(qr_img) = qr {
            drwcov(scratch, w, h, bg, qr_img, 0.0, None);
        } else {
            drwemp(scratch, w, h, bg);
        }
        return mkfrm(scratch, w, h);
    }

    // 远卡先画，近卡覆盖。
    let mut items: Vec<(f32, f32, &RgbaImage, Option<[u8; 3]>)> = Vec::with_capacity(slots.len());
    for (idx, cover, ring) in slots {
        let Some(cover) = cover else { continue; };
        let diff = *idx as f32 - cur;
        if diff.abs() <= 3.5 {
            items.push((diff.abs(), diff, cover.as_ref(), *ring));
        }
    }
    items.sort_by(|a, b| b.0.total_cmp(&a.0));

    for (_, diff, cover, ring) in items {
        drwcov(scratch, w, h, bg, cover, diff, ring);
    }

    mkfrm(scratch, w, h)
}

/// 将二维码图像嵌入手工白底正方形封面中，用于未登录首页 Cover 呈现。
pub(crate) fn mkqrcrd(qr: &RgbaImage) -> RgbaImage {
    let side = 240u32;
    let pad = 24u32;
    let inner = side - pad * 2;
    let mut card = RgbaImage::from_pixel(side, side, Rgba([255, 255, 255, 255]));
    let qw = qr.width().max(1);
    let qh = qr.height().max(1);
    for y in 0..inner {
        let sy = (y * qh / inner).min(qh - 1);
        for x in 0..inner {
            let sx = (x * qw / inner).min(qw - 1);
            card.put_pixel(x + pad, y + pad, *qr.get_pixel(sx, sy));
        }
    }
    card
}

/// 生成固定歌单（我喜欢 / 每日推荐 / 默认封面）的 432px 超清抗锯齿内建封面图。
pub(crate) fn mkart(kind: crate::deps::DeckKind) -> Arc<RgbaImage> {
    let side = 432u32;
    let fside = side as f32;
    let cov = |sd: f32| -> f32 { (0.5 - sd / 1.4).clamp(0.0, 1.0) };
    let mix = |bg: [f32; 3], fg: [f32; 3], a: f32| -> [f32; 3] {
        [
            bg[0] + (fg[0] - bg[0]) * a,
            bg[1] + (fg[1] - bg[1]) * a,
            bg[2] + (fg[2] - bg[2]) * a,
        ]
    };
    let img = RgbaImage::from_fn(side, side, |x, y| {
        let px = x as f32 + 0.5;
        let py = y as f32 + 0.5;
        let u = px / fside;
        let v = py / fside;
        let dx = px - fside * 0.5;
        let dy = py - fside * 0.5;
        let r = dx.hypot(dy);
        let mut c = match kind {
            crate::deps::DeckKind::Like => {
                let glow = (1.0 - (r / (fside * 0.65)).min(1.0)).powf(1.6);
                let mut col = [
                    224.0 - 38.0 * v + 18.0 * glow,
                    74.0 + 36.0 * (1.0 - u) - 18.0 * v + 16.0 * glow,
                    104.0 + 30.0 * u - 14.0 * v + 12.0 * glow,
                ];
                // 抗锯齿圆盘微纹底衬
                let ring = cov(((r - fside * 0.34).abs() - 1.2).max(0.0));
                col = mix(col, [255.0, 214.0, 222.0], ring * 0.18);
                // 心形SDF转像素距离。
                let s = fside * 0.25;
                let hx = dx / s;
                let hy = -dy / s + 0.12;
                let f = hx * hx + hy * hy - 0.62;
                let val = f * f * f - hx * hx * hy * hy * hy;
                let gx = 6.0 * f * f * hx - 2.0 * hx * hy * hy * hy;
                let gy = 6.0 * f * f * hy - 3.0 * hx * hx * hy * hy;
                let grad = gx.hypot(gy).max(1e-3);
                let sd_px = (val / grad) * s;
                col = mix(col, [255.0, 246.0, 248.0], cov(sd_px));
                col
            }
            crate::deps::DeckKind::DailyRecommend => {
                let glow = (1.0 - (r / (fside * 0.62)).min(1.0)).powf(1.5);
                let mut col = [
                    12.0 + 18.0 * (1.0 - v) + 22.0 * glow,
                    132.0 + 42.0 * (1.0 - v) + 34.0 * glow,
                    168.0 + 46.0 * u + 20.0 * glow,
                ];
                // 细腻哑光唱片槽与外圈柔光环
                let disc_outer = cov(r - fside * 0.365);
                col = mix(col, [7.0, 44.0, 58.0], disc_outer * 0.22);
                let rim = cov((r - fside * 0.365).abs() - 0.9);
                col = mix(col, [214.0, 248.0, 255.0], rim * 0.35);
                for frac in [0.315_f32, 0.265, 0.215] {
                    let g = cov((r - fside * frac).abs() - 0.7);
                    col = mix(col, [225.0, 250.0, 255.0], g * 0.24);
                }
                // 白色圆徽标与双音符。
                let badge = cov(r - fside * 0.145);
                col = mix(col, [246.0, 252.0, 255.0], badge);
                let note_s = fside * 0.088;
                let nx = dx / note_s + 0.11;
                let ny = dy / note_s - 0.047;
                let sd_seg = |px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32, rad: f32| -> f32 {
                    let pax = px - ax;
                    let pay = py - ay;
                    let bax = bx - ax;
                    let bay = by - ay;
                    let h = ((pax * bax + pay * bay) / (bax * bax + bay * bay)).clamp(0.0, 1.0);
                    (pax - bax * h).hypot(pay - bay * h) - rad
                };
                let sd_head = |px: f32, py: f32, cx: f32, cy: f32| -> f32 {
                    let qx = px - cx;
                    let qy = py - cy;
                    let (sa, ca) = (0.34_f32).sin_cos();
                    let u = qx * ca - qy * sa;
                    let v = qx * sa + qy * ca;
                    let rx = 0.23_f32;
                    let ry = 0.165_f32;
                    let k = (u / rx).hypot(v / ry);
                    let grad = (u / (rx * rx)).hypot(v / (ry * ry)).max(1e-4);
                    (k - 1.0) / grad
                };
                let d_lhead = sd_head(nx, ny, -0.34, 0.34);
                let d_rhead = sd_head(nx, ny, 0.34, 0.24);
                let d_lstem = sd_seg(nx, ny, -0.155, 0.32, -0.155, -0.44, 0.062);
                let d_rstem = sd_seg(nx, ny, 0.525, 0.22, 0.525, -0.54, 0.062);
                let d_beam = sd_seg(nx, ny, -0.155, -0.42, 0.525, -0.52, 0.105);
                let sd_note = d_lhead.min(d_rhead).min(d_lstem).min(d_rstem).min(d_beam) * note_s;
                col = mix(col, [0.0, 152.0, 184.0], cov(sd_note) * badge);
                col
            }
            _ => {
                let glow = (1.0 - (r / (fside * 0.65)).min(1.0)).powf(1.6);
                let mut col = [
                    54.0 + 24.0 * (1.0 - v) + 14.0 * glow,
                    78.0 + 24.0 * u + 12.0 * glow,
                    94.0 + 26.0 * (1.0 - v) + 14.0 * glow,
                ];
                let rim = cov((r - fside * 0.34).abs() - 1.1);
                let inner = cov((r - fside * 0.11).abs() - 1.1);
                col = mix(col, [210.0, 224.0, 232.0], (rim * 0.4 + inner * 0.5).min(1.0));
                col
            }
        };
        for ch in &mut c {
            *ch = ch.clamp(0.0, 255.0);
        }
        Rgba([c[0] as u8, c[1] as u8, c[2] as u8, 255])
    });
    Arc::new(img)
}

/// 队列为空时绘制居中唱片占位卡片与柔和倒影。
fn drwemp(buffer: &mut [u8], w: u32, h: u32, bg: [u8; 3]) {
    if w == 0 || h == 0 {
        return;
    }
    let dark = bg[0] < 128;
    let scale = w as f32 / 360.0;
    let size = 204.0 * scale;
    let half = size * 0.5;
    let cx = w as f32 * 0.5;
    let cy = 135.0 * scale;
    let x0 = (cx - half).floor().max(0.0) as i32;
    let x1 = (cx + half).ceil().min(w as f32 - 1.0) as i32;
    let y0 = (cy - half).floor().max(0.0) as i32;
    let y1 = (cy + half).ceil().min(h as f32 - 1.0) as i32;
    let radius = 12.0 * scale;
    let card = if dark { [41u8, 51, 59] } else { [245u8, 240, 235] };
    let groove = if dark { [58u8, 70, 80] } else { [223u8, 216, 208] };
    let rim = if dark { [72u8, 84, 94] } else { [255u8, 255, 255] };
    let r_outer = 62.0 * scale;
    let r_inner = 19.0 * scale;

    for sy in y0..=y1 {
        for sx in x0..=x1 {
            let lx = sx as f32 - (cx - half);
            let ly = sy as f32 - (cy - half);
            let dx = ((lx - half).abs() - (half - radius)).max(0.0);
            let dy = ((ly - half).abs() - (half - radius)).max(0.0);
            let edge = (radius - dx.hypot(dy) + 0.5).clamp(0.0, 1.0);
            if edge <= 0.0 {
                continue;
            }
            let dist = (sx as f32 - cx).hypot(sy as f32 - cy);
            let ring = ((dist - r_outer).abs() < 1.4 * scale)
                || ((dist - r_outer * 0.68).abs() < 0.9 * scale)
                || ((dist - r_inner).abs() < 1.4 * scale);
            let border = dx.hypot(dy) > radius - 1.2;
            let color = if border { rim } else if ring { groove } else { card };
            blend_pixel(buffer, w, sx as u32, sy as u32, color[0], color[1], color[2], (edge * 255.0) as u8);
        }
    }

    let refl_h = (26.0 * scale).max(1.0);
    let refl_top = (y1 as f32 + 2.0 * scale).ceil() as i32;
    let refl_bot = (refl_top as f32 + refl_h).floor().min(h as f32 - 1.0) as i32;
    let base_a = if dark { 0.22 } else { 0.14 };
    for sy in refl_top..=refl_bot {
        let t = ((sy - refl_top) as f32 / refl_h).clamp(0.0, 1.0);
        let fade = 1.0 - t;
        let a = base_a * fade * fade;
        if a <= 0.005 {
            continue;
        }
        for sx in x0..=x1 {
            let lx = sx as f32 - (cx - half);
            let ly = size - (sy - refl_top) as f32;
            let dx = ((lx - half).abs() - (half - radius)).max(0.0);
            let dy = ((ly - half).abs() - (half - radius)).max(0.0);
            let edge = (radius - dx.hypot(dy) + 0.5).clamp(0.0, 1.0);
            if edge > 0.0 {
                blend_pixel(buffer, w, sx as u32, sy as u32, card[0], card[1], card[2], (edge * a * 255.0) as u8);
            }
        }
    }
}

/// 按真实 3D 内向 V 形透视投影绘制专辑封面、抗锯齿圆角与地面倒影。
fn drwcov(buffer: &mut [u8], buf_w: u32, buf_h: u32, bg: [u8; 3], cover: &RgbaImage, diff: f32, ring: Option<[u8; 3]>) {
    if cover.width() == 0 || cover.height() == 0 || buf_w == 0 || buf_h == 0 {
        return;
    }
    let scale = buf_w as f32 / 360.0;
    let center_x = buf_w as f32 * 0.5;
    let center_y = 135.0 * scale;
    let unit_px = 108.0 * scale;
    let focal = 3.2_f32;

    let abs_d = diff.abs();
    let sgn = if diff < 0.0 { -1.0 } else if diff > 0.0 { 1.0 } else { 0.0 };
    let k = (abs_d / 0.85).min(1.0);
    let smooth = k * k * (3.0 - 2.0 * k);
    let stack = (abs_d - 1.0).max(0.0);

    // 侧卡内向V形透视。
    let theta = sgn * smooth * (52.0_f32).to_radians();
    let wx_center = sgn * (1.02 * smooth + 0.24 * stack);
    let wz_center = 0.90 * smooth + 0.14 * stack;
    let half_w = 1.0 - 0.12 * smooth - 0.03 * stack.min(2.0);
    let shade_factor = (1.0 - 0.15 * smooth - 0.06 * stack).clamp(0.58, 1.0);

    let (sin_t, cos_t) = theta.sin_cos();
    let wx0 = wx_center - half_w * cos_t;
    let wz0 = wz_center + half_w * sin_t;
    let wx1 = wx_center + half_w * cos_t;
    let wz1 = wz_center - half_w * sin_t;

    let inv_z0 = focal / (focal + wz0).max(0.4);
    let inv_z1 = focal / (focal + wz1).max(0.4);
    let x0 = center_x + wx0 * unit_px * inv_z0;
    let x1 = center_x + wx1 * unit_px * inv_z1;
    if x1 <= x0 + 0.5 {
        return;
    }

    let corner_uv = 10.0 / 216.0;
    let dark_bg = bg[0] < 128;

    let orig_w = cover.width() as f32;
    let orig_h = cover.height() as f32;
    let start_x = (x0.floor() as i32).max(0);
    let end_x = (x1.ceil() as i32).min(buf_w as i32 - 1);
    let refl_alpha = (if dark_bg { 0.28 } else { 0.18 }) * (1.0 - 0.12 * stack).clamp(0.45, 1.0);
    let refl_gap = 2.0 * scale;
    let ring_w = (4.0 * scale).max(2.0);
    let card_cx = (x0 + x1) * 0.5;
    let card_w2 = (x1 - x0) * 0.5;

    for sx in start_x..=end_x {
        let p = ((sx as f32 - x0) / (x1 - x0)).clamp(0.0, 1.0);
        let inv_z = inv_z0 * (1.0 - p) + inv_z1 * p;
        let u_norm = ((p * inv_z1) / inv_z.max(1e-4)).clamp(0.0, 1.0);
        let u = (u_norm * (orig_w - 1.0)).clamp(0.0, orig_w - 1.0);

        let half_h = half_w * unit_px * inv_z;
        let cur_yt = center_y - half_h;
        let cur_yb = center_y + half_h;
        let col_h = cur_yb - cur_yt;
        if col_h <= 0.5 {
            continue;
        }

        let start_y = (cur_yt.floor() as i32).max(0);
        let end_y = (cur_yb.ceil() as i32).min(buf_h as i32 - 1);
        let corner_px = corner_uv * col_h;
        let dx_uv = ((u_norm - 0.5).abs() - (0.5 - corner_uv)).max(0.0);

        for sy in start_y..=end_y {
            let ty = ((sy as f32 - cur_yt) / col_h).clamp(0.0, 1.0);
            let dy_uv = ((ty - 0.5).abs() - (0.5 - corner_uv)).max(0.0);
            let edge_cov = if dx_uv > 0.0 && dy_uv > 0.0 {
                let dist_px = (dx_uv * col_h).hypot(dy_uv * col_h);
                (corner_px - dist_px + 0.5).clamp(0.0, 1.0)
            } else {
                1.0
            };
            if edge_cov <= 0.0 {
                continue;
            }

            let v = (ty * (orig_h - 1.0)).clamp(0.0, orig_h - 1.0);
            let pixel = smpbl(cover, u, v);
            let mut r = (pixel[0] as f32 * shade_factor) as u8;
            let mut g = (pixel[1] as f32 * shade_factor) as u8;
            let mut b = (pixel[2] as f32 * shade_factor) as u8;
            let a = ((pixel[3] as f32) * edge_cov) as u8;
            if let Some([rr, rg, rb]) = ring {
                // 圆角矩形SDF描边。
                let qx = (sx as f32 - card_cx).abs() - (card_w2 - corner_px);
                let qy = (sy as f32 - center_y).abs() - (half_h - corner_px);
                let dist = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - corner_px;
                if dist >= -ring_w {
                    r = rr; g = rg; b = rb;
                }
            }
            blend_pixel(buffer, buf_w, sx as u32, sy as u32, r, g, b, a);
        }

        let refl_h = (col_h * 0.14).min(26.0 * scale);
        if refl_h > 1.0 {
            let ry0 = cur_yb + refl_gap;
            let r_start = (ry0.ceil() as i32).max(0);
            let r_end = ((ry0 + refl_h).floor() as i32).min(buf_h as i32 - 1);
            for sy in r_start..=r_end {
                let rt = ((sy as f32 - ry0) / refl_h).clamp(0.0, 1.0);
                let ty_src = (1.0 - rt * (refl_h / col_h)).clamp(0.0, 1.0);
                let dy_uv = ((ty_src - 0.5).abs() - (0.5 - corner_uv)).max(0.0);
                let edge_cov = if dx_uv > 0.0 && dy_uv > 0.0 {
                    let dist_px = (dx_uv * col_h).hypot(dy_uv * col_h);
                    (corner_px - dist_px + 0.5).clamp(0.0, 1.0)
                } else {
                    1.0
                };
                if edge_cov <= 0.0 {
                    continue;
                }
                let f = 1.0 - rt;
                let fade = refl_alpha * f * f * edge_cov;
                if fade <= 0.005 {
                    continue;
                }
                let v = (ty_src * (orig_h - 1.0)).clamp(0.0, orig_h - 1.0);
                let pixel = smpbl(cover, u, v);
                let r = (pixel[0] as f32 * shade_factor) as u8;
                let g = (pixel[1] as f32 * shade_factor) as u8;
                let b = (pixel[2] as f32 * shade_factor) as u8;
                let a = ((pixel[3] as f32) * fade) as u8;
                blend_pixel(buffer, buf_w, sx as u32, sy as u32, r, g, b, a);
            }
        }
    }
}

/// 将RGBA画布交给Slint原生共享像素缓冲。
pub(crate) fn mkfrm(bytes: &[u8], width: u32, height: u32) -> Arc<Frame> {
    Arc::new(Frame::clone_from_slice(bytes, width, height))
}

/// 双线性插值采样。
#[inline(always)]
fn smpbl(img: &RgbaImage, u: f32, v: f32) -> Rgba<u8> {
    let max_x = img.width() - 1;
    let max_y = img.height() - 1;
    let x0 = (u as u32).min(max_x);
    let y0 = (v as u32).min(max_y);
    let x1 = (x0 + 1).min(max_x);
    let y1 = (y0 + 1).min(max_y);

    let fx = u - x0 as f32;
    let fy = v - y0 as f32;
    let ifx = 1.0 - fx;
    let ify = 1.0 - fy;
    let w00 = ifx * ify;
    let w10 = fx * ify;
    let w01 = ifx * fy;
    let w11 = fx * fy;

    let p00 = img.get_pixel(x0, y0);
    let p10 = img.get_pixel(x1, y0);
    let p01 = img.get_pixel(x0, y1);
    let p11 = img.get_pixel(x1, y1);

    Rgba([
        (p00[0] as f32 * w00 + p10[0] as f32 * w10 + p01[0] as f32 * w01 + p11[0] as f32 * w11) as u8,
        (p00[1] as f32 * w00 + p10[1] as f32 * w10 + p01[1] as f32 * w01 + p11[1] as f32 * w11) as u8,
        (p00[2] as f32 * w00 + p10[2] as f32 * w10 + p01[2] as f32 * w01 + p11[2] as f32 * w11) as u8,
        (p00[3] as f32 * w00 + p10[3] as f32 * w10 + p01[3] as f32 * w01 + p11[3] as f32 * w11) as u8,
    ])
}

/// Alpha 混合写入像素。
#[inline(always)]
fn blend_pixel(buffer: &mut [u8], buf_w: u32, x: u32, y: u32, r: u8, g: u8, b: u8, a: u8) {
    if a == 0 {
        return;
    }
    let idx = ((y * buf_w + x) * 4) as usize;
    if idx + 3 >= buffer.len() {
        return;
    }

    if a == 255 {
        buffer[idx] = r;
        buffer[idx + 1] = g;
        buffer[idx + 2] = b;
        buffer[idx + 3] = 255;
    } else {
        let alpha = a as f32 / 255.0;
        let inv_alpha = 1.0 - alpha;

        buffer[idx] = (r as f32 * alpha + buffer[idx] as f32 * inv_alpha) as u8;
        buffer[idx + 1] = (g as f32 * alpha + buffer[idx + 1] as f32 * inv_alpha) as u8;
        buffer[idx + 2] = (b as f32 * alpha + buffer[idx + 2] as f32 * inv_alpha) as u8;
        buffer[idx + 3] = 255;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证内向V形透视、居中对齐、倒影、Single留白、快速中断及异步工作线程。
    #[test]
    fn tst_covfrm() {
        let mut engine = CoverFlowEngine::new(390, 300);
        let empty = engine.render_frame();
        assert_eq!(empty.as_bytes().len(), 390 * 300 * 4);
        assert_eq!(&empty.as_bytes()[..4], &[241, 235, 229, 255]);

        engine.add_cover(RgbaImage::from_pixel(2, 2, Rgba([255, 0, 0, 128])));
        engine.scroll_delta(-99.0);
        for _ in 0..100 {
            engine.tick();
        }
        assert_eq!(engine.selidx(), 0);
        let rendered = engine.render_frame();
        let offset = (146 * 390 + 195) * 4;
        assert_eq!(&rendered.as_bytes()[offset..offset + 4], &[248, 117, 114, 255]);
        assert!(!engine.tick());

        let mut sides = CoverFlowEngine::new(360, 274);
        for color in [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]] {
            sides.add_cover(RgbaImage::from_pixel(2, 2, Rgba(color)));
        }
        sides.sttgt(1);
        for _ in 0..120 {
            sides.tick();
        }
        let rendered = sides.render_frame();
        let pixels = rendered.as_bytes();
        let left = (135 * 360 + 45) * 4;
        let center = (135 * 360 + 180) * 4;
        let right = (135 * 360 + 315) * 4;
        let refl = (252 * 360 + 180) * 4;
        // 核验左侧V形透视。
        let left_outer_top = (50 * 360 + 28) * 4;
        let left_inner_top = (50 * 360 + 66) * 4;
        assert!(pixels[left] > 180 && pixels[left + 2] < 20, "left tilted cover missing");
        assert!(pixels[left_outer_top] > 180 && pixels[left_outer_top + 1] < 20, "left outer edge must stay taller (facing inward)");
        assert!(pixels[left_inner_top + 1] > 180, "left inner edge must taper shorter toward center");
        assert_eq!(&pixels[center..center + 4], &[0, 255, 0, 255], "center cover mismatch");
        assert!(pixels[right + 2] > 180 && pixels[right] < 20, "right tilted cover missing");
        assert!(pixels[refl + 1] > pixels[refl] + 10, "floor reflection missing under center cover");

        // 单卡布局拖拽回弹。
        let mut single = CoverFlowEngine::new(360, 274);
        single.stwin(vec![(0, Some(Arc::new(RgbaImage::from_pixel(2, 2, Rgba([0, 255, 0, 255])))), None)], 1, 0);
        let frame = single.render_frame();
        for x in [20, 340] {
            assert_eq!(&frame.as_bytes()[(135 * 360 + x) * 4..(135 * 360 + x) * 4 + 4], &[241, 235, 229, 255]);
        }
        assert_eq!(&frame.as_bytes()[(135 * 360 + 180) * 4..(135 * 360 + 180) * 4 + 4], &[0, 255, 0, 255]);
        single.drgstrt();
        single.drgmv(-80.0);
        assert!(single.target_offset > 0.0);
        assert_eq!(single.drgrel(6.0), Some(0));
        for _ in 0..120 {
            single.tick();
        }
        assert!((single.current_offset - 0.0).abs() < 0.01);

        // 连续切歌平滑接续。
        let mut rapid = CoverFlowEngine::new(360, 274);
        let imgs: Vec<_> = (0..5)
            .map(|i| (i, Some(Arc::new(RgbaImage::from_pixel(2, 2, Rgba([40 * i as u8, 120, 200, 255])))), None))
            .collect();
        rapid.stwin(imgs.clone(), 5, 0);
        rapid.stwin(imgs.clone(), 5, 1);
        for _ in 0..3 {
            rapid.flwstep(16.0);
        }
        let mid = rapid.current_offset;
        assert!(mid > 0.1 && mid < 1.0);
        rapid.stwin(imgs.clone(), 5, 3);
        rapid.flwstep(16.0);
        assert!(rapid.current_offset > mid, "rapid skip jumped backward instead of continuing");

        // 未登录渲染二维码卡。
        let mut qr_engine = CoverFlowEngine::new(360, 274);
        qr_engine.qr_cover = Some(Arc::new(mkqrcrd(&RgbaImage::from_pixel(64, 64, Rgba([0, 0, 0, 255])))));
        let qr_frame = qr_engine.render_frame();
        assert_eq!(&qr_frame.as_bytes()[(135 * 360 + 180) * 4..(135 * 360 + 180) * 4 + 4], &[0, 0, 0, 255]);
        assert_eq!(&qr_frame.as_bytes()[(42 * 360 + 86) * 4..(42 * 360 + 86) * 4 + 4], &[255, 255, 255, 255]);

        // 登录后flow停靠日推。
        let mut login_flow = CoverFlowEngine::new(360, 274);
        login_flow.stwin(imgs.clone(), 5, 1);
        login_flow.flwlogin(1);
        assert!(login_flow.current_offset < 0.0);
        for _ in 0..160 {
            login_flow.flwstep(16.0);
        }
        assert!((login_flow.current_offset - 1.0).abs() < 0.01);

        // 核验默认封面尺寸。
        let daily_art = mkart(crate::deps::DeckKind::DailyRecommend);
        assert_eq!(daily_art.width(), 432);
        assert_eq!(daily_art.height(), 432);
        assert!(daily_art.get_pixel(216, 216)[0] > 235);
        let note_px = daily_art.get_pixel(216 - 17, 216 + 15);
        assert!(note_px[0] < 30 && note_px[2] > 160, "note head should render cyan inside white badge");
        let (mut sx, mut sy, mut cnt) = (0f64, 0f64, 0f64);
        for y in 160..272 {
            for x in 160..272 {
                let d = ((x as f64 - 215.5).powi(2) + (y as f64 - 215.5).powi(2)).sqrt();
                if d >= 56.0 { continue; }
                let p = daily_art.get_pixel(x, y);
                if p[0] < 120 {
                    sx += x as f64; sy += y as f64; cnt += 1.0;
                }
            }
        }
        assert!((sx / cnt - 215.5).abs() < 1.0, "note visual X centroid off-center: {:.2}", sx / cnt);
        assert!((sy / cnt - 215.5).abs() < 1.0, "note visual Y centroid off-center: {:.2}", sy / cnt);

        // 渲染线程独立生成帧。
        let worker = FlwWrkr::nw(360, 274, [241, 235, 229]);
        worker.stwin(imgs, 5, 2);
        let deadline = Instant::now() + std::time::Duration::from_secs(2);
        while Instant::now() < deadline {
            if let Some(f) = worker.pllfrm() {
                if f.as_bytes()[(135 * 360 + 180) * 4 + 2] > 180 {
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let f = worker.pllfrm().unwrap();
        assert!(f.as_bytes()[(135 * 360 + 180) * 4 + 2] > 180);
    }
}
