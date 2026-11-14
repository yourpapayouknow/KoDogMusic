// 线程执行核心命令。
use crate::deps::*;

/// 核心命令线程、共享快照与二维码缓存。
pub struct Worker {
    tx: Option<std::sync::mpsc::SyncSender<AppCommand>>,
    pub state: Arc<Mutex<UiSnapshot>>,
    pub busy: Arc<std::sync::atomic::AtomicBool>,
    pub qr: Arc<Mutex<Option<Arc<Frame>>>>,
    pub qr_img: Arc<Mutex<Option<Arc<RgbaImage>>>>,
    pub art: Arc<crate::artwork::ArtCache>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Worker {
    /// 启动独立核心并发布真实状态。
    pub fn nw(root: PathBuf) -> Self {
        let (tx, rx) = std::sync::mpsc::sync_channel(32);
        let state = Arc::new(Mutex::new(UiSnapshot::default()));
        let shared = state.clone();
        let busy = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let pending = busy.clone();
        let qr = Arc::new(Mutex::new(None));
        let qrframe = qr.clone();
        let qr_img = Arc::new(Mutex::new(None));
        let qrpix = qr_img.clone();
        let client = KugouClient::opn(root.join("secrets.dat"));
        let art = Arc::new(crate::artwork::ArtCache::nw(client.as_ref().ok().cloned().unwrap_or_else(KugouClient::nw), root.clone()));
        let images = art.clone();
        let thread = std::thread::spawn(move || {
            let storage = match DatabaseStorage::opn(root.join("library.db"), root.join("config.toml")) {
                Ok(storage) => Arc::new(storage),
                Err(error) => {
                    shared.lock().unwrap().error = Some(error.to_string());
                    pending.store(false, std::sync::atomic::Ordering::Release);
                    return;
                }
            };
            let client = match client {
                Ok(client) => client,
                Err(error) => { shared.lock().unwrap().error = Some(error.to_string()); pending.store(false, std::sync::atomic::Ordering::Release); return; }
            };
            let _ = client.regdev();
            let mut core = Coordinator::nw(
                Arc::new(KugouCatalog::nw(client.clone())), Arc::new(KugouAuth::nw(client.clone())),
                Arc::new(KugouLibrary::nw(client.clone())), Arc::new(KugouStream::nw(client)),
                Arc::new(AudioController::nw()), storage,
            );
            if matches!(core.ssht().account, AccountState::Guest) {
                core.hndlcmd(AppCommand::AuthLogin);
            } else {
                core.hndlcmd(AppCommand::LoadCoverFlow);
            }
            let mut qrkey = String::new();
            let mut started = std::time::Instant::now();
            let mut poll = std::time::Instant::now();
            let mut account = match &core.ssht().account { AccountState::Guest => None, AccountState::Authorized { user_id, .. } => Some(user_id.clone()) };
            let (lyrtx, lyrrx) = std::sync::mpsc::sync_channel::<(TrackId, std::result::Result<Vec<LyricLine>, CoreError>)>(1);
            let mut lyrjob: Option<std::thread::JoinHandle<()>> = None;
            let mut requested = None;
            // 每轮刷新快照与命令。
            loop {
                images.config(&root, &core.settings.cache);
                if lyrjob.as_ref().is_some_and(|job| job.is_finished()) {
                    let failed = lyrjob.take().unwrap().join().is_err();
                    if let Ok((id, result)) = lyrrx.try_recv() {
                        match result {
                            Ok(lines) => core.stlyrc(&id, lines),
                            Err(error) => {
                                if core.ssht().current.as_ref().is_some_and(|track| track.id == id) && core.snapshot.error.is_none() {
                                    core.snapshot.error = Some(error.to_string());
                                }
                            }
                        }
                    } else if failed {
                        core.snapshot.error = Some("歌词读取失败".into());
                    }
                }
                if !core.settings.lyrics.desktop_enabled {
                    requested = None;
                } else if let Some(track) = &core.ssht().current {
                    if requested.as_ref() != Some(&track.id) && lyrjob.is_none() {
                        let id = track.id.clone();
                        requested = Some(id.clone());
                        let sender = lyrtx.clone();
                        let catalog = core.catalog.clone();
                        lyrjob = Some(std::thread::spawn(move || { let result = catalog.lyrc(&id); let _ = sender.send((id, result)); }));
                    }
                }
                let current = match &core.ssht().account { AccountState::Guest => None, AccountState::Authorized { user_id, .. } => Some(user_id.clone()) };
                if account != current {
                    if current.is_some() && core.ssht().decks.is_empty() {
                        core.hndlcmd(AppCommand::LoadCoverFlow);
                    }
                    account = current;
                }
                if let Some(qr) = &core.snapshot.login_qr {
                    if qr.key != qrkey {
                        qrkey = qr.key.clone();
                        started = std::time::Instant::now();
                        poll = started;
                        match mkqr_rgba(&qr.link) {
                            Ok(rgba) => {
                                *qrframe.lock().unwrap() = Some(qrfrm(&rgba));
                                *qrpix.lock().unwrap() = Some(Arc::new(rgba));
                            }
                            Err(error) => { core.snapshot.error = Some(error); core.snapshot.login_qr = None; }
                        }
                    }
                } else {
                    qrkey.clear();
                    *qrframe.lock().unwrap() = None;
                    *qrpix.lock().unwrap() = None;
                }
                *shared.lock().unwrap() = core.ssht().clone();
                pending.store(false, std::sync::atomic::Ordering::Release);
                match rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(command) => {
                        pending.store(true, std::sync::atomic::Ordering::Release);
                        shared.lock().unwrap().error = None;
                        if matches!(command, AppCommand::QueueRemove(_)) { requested = None; }
                        core.hndlcmd(command);
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        if let Some(qr) = &mut core.snapshot.login_qr {
                            if started.elapsed() > Duration::from_secs(180) { qr.stage = QrStage::Expired; }
                        }
                        if core.snapshot.login_qr.as_ref().is_some_and(|qr| qr.stage != QrStage::Expired) && poll.elapsed() >= Duration::from_secs(2) {
                            poll = std::time::Instant::now();
                            pending.store(true, std::sync::atomic::Ordering::Release);
                            core.hndlcmd(AppCommand::AuthPoll);
                        } else {
                            core.tick();
                        }
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
            core.hndlcmd(AppCommand::Stop);
        });
        Self { tx: Some(tx), state, busy, qr, qr_img, art, thread: Some(thread) }
    }

    /// 有界投递命令并反馈后台繁忙。
    pub fn snd(&self, command: AppCommand) -> bool {
        self.tx.as_ref().is_some_and(|tx| tx.try_send(command).is_ok())
    }
}

/// 使用已有二维码库生成带静区的黑白 RGBA 图像。
fn mkqr_rgba(link: &str) -> std::result::Result<RgbaImage, String> {
    let code = QrCode::new(link.as_bytes()).map_err(|_| "无法生成二维码".to_string())?;
    let width = code.width() as u32;
    let scale = (352 / (width + 8)).max(1);
    let side = (width + 8) * scale;
    Ok(RgbaImage::from_fn(side, side, |x, y| {
        let (column, row) = (x / scale, y / scale);
        let dark = column >= 4 && row >= 4 && column < width + 4 && row < width + 4
            && code[((column - 4) as usize, (row - 4) as usize)] == QrColor::Dark;
        if dark { Rgba([0, 0, 0, 255]) } else { Rgba([255, 255, 255, 255]) }
    }))
}

/// 将二维码位图转换为共享帧。
fn qrfrm(rgba: &RgbaImage) -> Arc<Frame> {
    crate::coverflow::mkfrm(rgba.as_raw(), rgba.width(), rgba.width())
}

/// 使用已有二维码库生成带静区的黑白纹理。
#[cfg(test)]
fn mkqr(link: &str) -> std::result::Result<Arc<Frame>, String> {
    Ok(qrfrm(&mkqr_rgba(link)?))
}

impl Drop for Worker {
    /// 关闭命令通道并回收播放与数据库资源。
    fn drop(&mut self) {
        self.tx.take();
        if let Some(thread) = self.thread.take() { let _ = thread.join(); }
    }
}

#[cfg(test)]
mod tests {
    /// 验证二维码静区、黑白通道及不同内容的纹理。
    #[test]
    fn tst_qrimg() {
        let first = super::mkqr("https://h5.kugou.com/?qrcode=synthetic-first").unwrap();
        let second = super::mkqr("https://h5.kugou.com/?qrcode=synthetic-second").unwrap();
        let pixels = first.as_bytes();
        assert_eq!(&pixels[..4], &[255, 255, 255, 255]);
        assert!(pixels.chunks_exact(4).all(|p| p[0] == p[1] && p[1] == p[2] && p[3] == 255));
        assert!(pixels.chunks_exact(4).any(|p| p[0] == 0));
        assert_ne!(pixels, second.as_bytes());
    }
}
