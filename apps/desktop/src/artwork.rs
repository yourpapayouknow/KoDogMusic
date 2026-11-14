// 封面与头像共享缓存。
use crate::deps::*;
use std::collections::{HashMap, HashSet};

/// 封面像素与其缩略图帧。
#[derive(Clone)]
pub struct Artwork { pub pixels: Arc<RgbaImage>, pub frame: Option<Arc<Frame>> }
/// 单条缓存内容、访问时间与占用字节。
struct Entry { value: Artwork, used: std::time::Instant, cost: usize }
/// 有界缓存条目、在途请求与失败记录。
#[derive(Default)]
struct Cache { entries: HashMap<(String, u32), Entry>, pending: HashSet<(String, u32)>, failed: HashMap<(String, u32), std::time::Instant>, bytes: usize, retired: Vec<Arc<Frame>> }
/// 有界封面解码缓存与加载线程。
pub struct ArtCache {
    shared: Arc<Mutex<Cache>>,
    policy: Arc<Mutex<(PathBuf, usize, u64)>>,
    sender: Option<std::sync::mpsc::SyncSender<(String, u32)>>,
    thread: Option<std::thread::JoinHandle<()>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
}

impl ArtCache {
    /// 独立加载图片，GUI 与播放命令均不等待网络。
    pub fn nw(client: KugouClient, root: PathBuf) -> Self {
        let shared = Arc::new(Mutex::new(Cache::default()));
        let state = shared.clone();
        let policy = Arc::new(Mutex::new((root.join("artwork"), 3 * 1024 * 1024, 2048 * 1024 * 1024)));
        let config = policy.clone();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stopped = stop.clone();
        let (sender, receiver) = std::sync::mpsc::sync_channel::<(String, u32)>(8);
        let thread = std::thread::spawn(move || {
            while let Ok(key) = receiver.recv() {
                if stopped.load(std::sync::atomic::Ordering::Acquire) { break; }
                let (directory, budget, disk) = config.lock().unwrap().clone();
                let value = load(&client, &key.0, key.1, &directory, disk);
                let mut cache = state.lock().unwrap();
                cache.pending.remove(&key);
                match value {
                    Ok(pixels) => {
                        // 大图投影，小图用共享帧。
                        let thumbnail = key.1 <= 128;
                        let cost = pixels.len() * if thumbnail { 2 } else { 1 };
                        let frame = thumbnail.then(|| mkthumb(&pixels, key.1 == 64));
                        while cache.bytes + cost > budget || cache.entries.len() >= 64 {
                            let Some(oldest) = cache.entries.iter().min_by_key(|(_, item)| item.used).map(|(key, _)| key.clone()) else { break; };
                            if let Some(entry) = cache.entries.remove(&oldest) {
                                cache.bytes -= entry.cost;
                                if let Some(frame) = entry.value.frame { cache.retired.push(frame); }
                            }
                        }
                        if cost <= budget {
                            cache.bytes += cost;
                            cache.entries.insert(key, Entry { value: Artwork { pixels: Arc::new(pixels), frame }, used: std::time::Instant::now(), cost });
                        }
                    }
                    Err(_) => {
                        if cache.failed.len() >= 64 { cache.failed.retain(|_, time| time.elapsed() < Duration::from_secs(60)); }
                        if cache.failed.len() >= 64 { cache.failed.clear(); }
                        cache.failed.insert(key, std::time::Instant::now());
                    }
                }
            }
        });
        Self { shared, policy, sender: Some(sender), thread: Some(thread), stop }
    }

    /// 更新已确认的缓存目录与预算。
    pub fn config(&self, root: &Path, settings: &CacheSettings) {
        let directory = if settings.directory.as_os_str().is_empty() { root.join("artwork") } else { settings.directory.join("KoDogLite-artwork") };
        *self.policy.lock().unwrap() = (directory, settings.max_artwork_ram_mib.clamp(1, 3) as usize * 1024 * 1024, settings.max_disk_mib.saturating_mul(1024 * 1024));
    }

    /// 取出淘汰图片并及时释放共享缓冲。
    pub fn rtrd(&self) -> Vec<Arc<Frame>> { std::mem::take(&mut self.shared.lock().unwrap().retired) }

    /// 仅从内存缓存读取已解码纹理，不触发新的后台加载。
    pub fn peek(&self, url: &str, size: u32) -> Option<Artwork> {
        if url.is_empty() { return None; }
        let key = (url.to_string(), size.clamp(1, 512));
        let mut cache = self.shared.lock().unwrap();
        if let Some(entry) = cache.entries.get_mut(&key) {
            entry.used = std::time::Instant::now();
            return Some(entry.value.clone());
        }
        None
    }

    /// 命中时复用同一纹理，未命中时只投递一次有界请求。
    pub fn get(&self, url: &str, size: u32) -> Option<Artwork> {
        if url.is_empty() { return None; }
        let key = (url.to_string(), size.clamp(1, 512));
        let mut cache = self.shared.lock().unwrap();
        if let Some(entry) = cache.entries.get_mut(&key) { entry.used = std::time::Instant::now(); return Some(entry.value.clone()); }
        if cache.pending.contains(&key) || cache.failed.get(&key).is_some_and(|time| time.elapsed() < Duration::from_secs(60)) { return None; }
        if self.sender.as_ref().is_some_and(|sender| sender.try_send(key.clone()).is_ok()) { cache.pending.insert(key); }
        None
    }
}

/// 在加载线程生成圆形头像或圆角封面缓冲。
fn mkthumb(pixels: &RgbaImage, circle: bool) -> Arc<Frame> {
    let side = pixels.width().min(pixels.height());
    let mut square = image::imageops::crop_imm(pixels, (pixels.width()-side)/2, (pixels.height()-side)/2, side, side).to_image();
    let half = side as f32 / 2.;
    let radius = if circle { half } else { side as f32 / 6. };
    for (x,y,pixel) in square.enumerate_pixels_mut() {
        let dx = ((x as f32 + 0.5 - half).abs() - (half-radius)).max(0.);
        let dy = ((y as f32 + 0.5 - half).abs() - (half-radius)).max(0.);
        pixel.0[3] = (pixel.0[3] as f32 * (radius - dx.hypot(dy) + 0.5).clamp(0.,1.)) as u8;
    }
    crate::coverflow::mkfrm(&square.into_raw(), side, side)
}

impl Drop for ArtCache {
    /// 停止后台资源读取并释放纹理缓存。
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
        self.sender.take();
        if let Some(thread) = self.thread.take() { let _ = thread.join(); }
    }
}

/// 读取自有缓存，损坏时重新获取并限制磁盘占用。
fn load(client: &KugouClient, url: &str, side: u32, directory: &Path, budget: u64) -> std::result::Result<RgbaImage, CoreError> {
    use std::io::{Read, Write};
    let filename = format!("kodog-art-{:x}.bin", Md5::digest(url.as_bytes()));
    let path = directory.join(filename);
    if let Ok(file) = std::fs::File::open(&path) {
        let mut bytes = Vec::new();
        if file.take(4 * 1024 * 1024 + 1).read_to_end(&mut bytes).is_ok() && bytes.len() <= 4 * 1024 * 1024 {
            if let Ok(image) = ldimage(&bytes, side) { return Ok(image); }
        }
    }
    let bytes = client.blob(url, 4 * 1024 * 1024)?;
    let image = ldimage(&bytes, side)?;
    if budget >= bytes.len() as u64 && std::fs::create_dir_all(directory).is_ok() {
        let temp = directory.join(format!("kodog-art-{}.tmp", Uuid::new_v4()));
        if let Ok(mut file) = std::fs::OpenOptions::new().create_new(true).write(true).open(&temp) {
            if file.write_all(&bytes).and_then(|_| file.sync_all()).is_ok() { drop(file); let _ = std::fs::rename(&temp, &path); }
            let _ = std::fs::remove_file(&temp);
        }
        prune(directory, budget);
    }
    Ok(image)
}

/// 仅淘汰客户端自有命名的普通图片缓存文件。
fn prune(directory: &Path, budget: u64) {
    let Ok(entries) = std::fs::read_dir(directory) else { return; };
    let mut files = Vec::new(); let mut total = 0u64;
    for entry in entries.flatten() {
        let name = entry.file_name(); let name = name.to_string_lossy(); let name = name.as_bytes();
        if name.len() != 46 || !name.starts_with(b"kodog-art-") || !name.ends_with(b".bin") || !name[10..42].iter().all(|c| c.is_ascii_hexdigit()) { continue; }
        if let Ok(meta) = entry.metadata() { if meta.is_file() && !entry.file_type().is_ok_and(|kind| kind.is_symlink()) { total = total.saturating_add(meta.len()); files.push((meta.modified().ok(), meta.len(), entry.path())); } }
    }
    files.sort_by_key(|file| file.0);
    for (_, size, path) in files { if total <= budget { break; } if std::fs::remove_file(path).is_ok() { total = total.saturating_sub(size); } }
}

#[cfg(test)]
mod tests {
    /// 圆角与圆形缓冲保持中心不透明、四角透明。
    #[test]
    fn tst_thumb() {
        use super::*;
        let image = RgbaImage::from_pixel(80,64,Rgba([20,40,60,255]));
        for circle in [false,true] {
            let thumb = mkthumb(&image,circle);
            assert_eq!(thumb.width(),64);
            assert_eq!(thumb.as_bytes()[3],0);
            assert_eq!(thumb.as_bytes()[(32*64+32)*4+3],255);
        }
    }

    /// 淘汰缩略图时同时回收纹理，并保持缓存预算上限。
    #[test]
    fn tst_artlim() {
        use super::*;
        let root = std::env::temp_dir().join(format!("kodog-art-limit-{}", Uuid::new_v4()));
        std::fs::create_dir_all(root.join("artwork")).unwrap();
        let cache = ArtCache::nw(KugouClient::nw(), root.clone());
        let settings = CacheSettings { max_artwork_ram_mib: 64, ..Default::default() };
        cache.config(&root, &settings);
        assert_eq!(cache.policy.lock().unwrap().1, 3 * 1024 * 1024);
        let pixels = Arc::new(RgbaImage::from_pixel(2, 2, Rgba([20, 40, 60, 255])));
        let frame = crate::coverflow::mkfrm(pixels.as_raw(), 2, 2);
        {
            let mut data = cache.shared.lock().unwrap();
            for index in 0..64 { data.entries.insert((format!("old-{index}"), 80), Entry { value: Artwork { pixels: pixels.clone(), frame: Some(frame.clone()) }, used: std::time::Instant::now(), cost: 32 }); }
            data.bytes = 64 * 32;
        }
        let url = "cache-only-test";
        pixels.save_with_format(root.join("artwork").join(format!("kodog-art-{:x}.bin", Md5::digest(url.as_bytes()))), image::ImageFormat::Png).unwrap();
        let limit = std::time::Instant::now() + Duration::from_secs(2);
        while cache.get(url, 80).is_none() { assert!(std::time::Instant::now() < limit); std::thread::sleep(Duration::from_millis(5)); }
        assert_eq!(cache.shared.lock().unwrap().entries.len(), 64);
        assert_eq!(cache.rtrd().len(), 1);
        assert!(cache.rtrd().is_empty());
        drop(cache);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 缓存淘汰只处理客户端自有文件。
    #[test]
    fn tst_artprn() {
        use super::*;
        let root = std::env::temp_dir().join(format!("kodog-art-{}", Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let owned = root.join("kodog-art-0123456789abcdef0123456789abcdef.bin");
        let user = root.join("photo.bin");
        std::fs::write(&owned, [0; 10]).unwrap(); std::fs::write(&user, [0; 10]).unwrap();
        super::prune(&root, 0);
        assert!(!owned.exists()); assert!(user.exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
