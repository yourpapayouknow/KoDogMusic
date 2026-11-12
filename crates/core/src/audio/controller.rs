use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use crate::application::ports::PlaybackPort;
use crate::audio::clock::PlaybackClock;
use crate::audio::convert::Conv;
use crate::audio::decoder::Decoder;
use crate::audio::output::Output;
use crate::audio::ring::RealtimeRing;
use crate::audio::source::{FileSource, HttpSource};
use crate::deps::symphonia::core::formats::probe::Hint;
use crate::deps::symphonia::core::io::{MediaSource, MediaSourceStream, MediaSourceStreamOptions};
use crate::domain::error::AppError;
use crate::domain::playback::{PlaybackProgress, PlaybackState};
use crate::domain::track::ResolvedSource;

/// 输出缓冲目标时长（秒），用于吸收网络分段拉取与解码抖动。
const BUFFER_SECS: usize = 1;
/// 输出缓冲最小样本数。
const RING_MIN: usize = 64 * 1024;

/// 音频控制器
pub struct AudioController {
    ring: Mutex<Option<Arc<RealtimeRing>>>,
    clock: Arc<PlaybackClock>,
    state: Arc<AtomicU32>,
    volume: Arc<AtomicU32>,
    output: Arc<Mutex<Option<Output>>>,
    worker: Mutex<Option<JoinHandle<()>>>,
    stop_signal: Arc<AtomicBool>,
    seek_signal: Arc<AtomicU64>,
    total_duration_ms: Arc<AtomicU64>,
}

impl AudioController {
    /// 构造音频控制器
    pub fn nw() -> Self {
        let clock = Arc::new(PlaybackClock::nw());
        Self {
            ring: Mutex::new(None),
            clock,
            state: Arc::new(AtomicU32::new(PlaybackState::Idle as u32)),
            volume: Arc::new(AtomicU32::new(0.5f32.to_bits())),
            output: Arc::new(Mutex::new(None)),
            worker: Mutex::new(None),
            stop_signal: Arc::new(AtomicBool::new(false)),
            seek_signal: Arc::new(AtomicU64::new(u64::MAX)),
            total_duration_ms: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl Default for AudioController {
    /// 构造默认音频控制器。
    fn default() -> Self {
        Self::nw()
    }
}

impl PlaybackPort for AudioController {
    /// 停止旧会话后创建真实输出与解码任务。
    fn ply(&self, src: ResolvedSource) -> Result<(), AppError> {
        self.stp();

        let def_cfg = Output::defcfg()?;
        // 缓冲按输出格式定时长。
        let ring = Arc::new(RealtimeRing::nw(
            (def_cfg.sample_rate as usize * def_cfg.channels as usize)
                .saturating_mul(BUFFER_SECS)
                .max(RING_MIN),
        ));
        *self.ring.lock().unwrap() = Some(ring.clone());
        let out = Output::nw(ring.clone(), def_cfg, self.volume.clone())?;
        let played = out.played_frames.clone();
        *self.output.lock().unwrap() = Some(out);
        let out_handle = self.output.clone();
        let clock = self.clock.clone();
        let state = self.state.clone();
        let stop = self.stop_signal.clone();
        let seek_sig = self.seek_signal.clone();
        let total_dur = self.total_duration_ms.clone();

        ring.clr();
        stop.store(false, Ordering::Release);
        seek_sig.store(u64::MAX, Ordering::Release);
        state.store(PlaybackState::Buffering as u32, Ordering::Release);

        let worker = std::thread::spawn(move || {
            let remote = src.url_or_local.starts_with("http://") || src.url_or_local.starts_with("https://");
            let mss: Result<Box<dyn MediaSource>, AppError> = if remote {
                HttpSource::nw(&src.url_or_local).map(|s| Box::new(s) as Box<dyn MediaSource>)
            } else {
                FileSource::opn(&src.url_or_local).map(|s| Box::new(s) as Box<dyn MediaSource>)
            };

            let media_source = match mss {
                Ok(ms) => ms,
                Err(e) => {
                    crate::deps::tracing::error!(error = %e, "打开音频媒体流失败");
                    state.store(PlaybackState::Failed as u32, Ordering::Release);
                    return;
                }
            };

            let stream = MediaSourceStream::new(media_source, MediaSourceStreamOptions::default());
            let mut hint = Hint::new();
            if let Some(ref h) = src.codec_hint {
                hint.with_extension(h);
            }

            let mut decoder = match Decoder::frmmss(stream, &hint) {
                Ok(d) => d,
                Err(e) => {
                    crate::deps::tracing::error!(error = %e, "创建解码器失败");
                    state.store(PlaybackState::Failed as u32, Ordering::Release);
                    return;
                }
            };

            if let Some(sec) = decoder.totdursec() {
                total_dur.store((sec * 1000.0) as u64, Ordering::Release);
            }

            let mut conv = match Conv::nw(decoder.spec().sample_rate, decoder.spec().channels, &def_cfg) {
                Ok(c) => c,
                Err(e) => {
                    crate::deps::tracing::error!(error = %e, "创建格式转换器失败");
                    state.store(PlaybackState::Failed as u32, Ordering::Release);
                    return;
                }
            };

            let _ = state.compare_exchange(
                PlaybackState::Buffering as u32,
                PlaybackState::Playing as u32,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
            let mut pcm_raw = Vec::with_capacity(4096);
            let mut pcm_out = Vec::with_capacity(4096);
            // 预填充一半再启动。
            let prefill = (ring.cap() / 2).max(1);
            let mut started = false;
            let mut start = || {
                if !started {
                    if let Some(out) = out_handle.lock().unwrap().as_ref() { let _ = out.ply(); }
                    started = true;
                }
            };

            while !stop.load(Ordering::Acquire) {
                let target_seek = seek_sig.swap(u64::MAX, Ordering::AcqRel);
                if target_seek != u64::MAX {
                    let sec = target_seek as f64 / 1000.0;
                    if let Ok(landed) = decoder.sk(sec) {
                        ring.clr();
                        played.store(0, Ordering::Release);
                        clock.stbs((landed * 1000.0) as u64);
                        if let Ok(new_conv) = Conv::nw(decoder.spec().sample_rate, decoder.spec().channels, &def_cfg) {
                            conv = new_conv;
                        }
                    }
                }

                // 暂停时保留会话与定位信号。
                if state.load(Ordering::Acquire) == PlaybackState::Paused as u32 {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                    continue;
                }

                match decoder.nxtchk(&mut pcm_raw) {
                    Ok(true) => {
                        pcm_out.clear();
                        if conv.cnvr(&pcm_raw, &mut pcm_out).is_ok() {
                            wrtpcm(&ring, &pcm_out, def_cfg.channels, &stop, &seek_sig);
                        }
                        if ring.buflen() >= prefill { start(); }
                    }
                    Ok(false) => {
                        pcm_out.clear();
                        if conv.flsh(&mut pcm_out).is_ok() {
                            wrtpcm(&ring, &pcm_out, def_cfg.channels, &stop, &seek_sig);
                        }
                        start();
                        // 缓冲清空后发布结束。
                        while !ring.isempty() && !stop.load(Ordering::Acquire) && seek_sig.load(Ordering::Acquire) == u64::MAX {
                            std::thread::sleep(std::time::Duration::from_millis(10));
                        }
                        if seek_sig.load(Ordering::Acquire) != u64::MAX { continue; }
                        if !stop.load(Ordering::Acquire) {
                            state.store(PlaybackState::Ended as u32, Ordering::Release);
                        }
                        break;
                    }
                    Err(e) => {
                        crate::deps::tracing::error!(error = %e, "解码循环异常");
                        state.store(PlaybackState::Failed as u32, Ordering::Release);
                        break;
                    }
                }
            }
        });

        *self.worker.lock().unwrap() = Some(worker);
        Ok(())
    }

    /// 暂停输出并保留播放会话。
    fn ps(&self) {
        self.state.store(PlaybackState::Paused as u32, Ordering::Release);
        if let Some(ref out) = *self.output.lock().unwrap() {
            let _ = out.ps();
        }
    }

    /// 恢复当前输出会话。
    fn rsm(&self) {
        if let Some(ref out) = *self.output.lock().unwrap() {
            let _ = out.ply();
            self.state.store(PlaybackState::Playing as u32, Ordering::Release);
        }
    }

    /// 取消并回收旧解码线程及输出流。
    fn stp(&self) {
        self.stop_signal.store(true, Ordering::Release);
        if let Some(worker) = self.worker.lock().unwrap().take() {
            let _ = worker.join();
        }
        if let Some(out) = self.output.lock().unwrap().take() {
            let _ = out.ps();
        }
        if let Some(ring) = self.ring.lock().unwrap().as_ref() {
            ring.clr();
        }
        self.clock.stbs(0);
        self.total_duration_ms.store(0, Ordering::Release);
        self.state.store(PlaybackState::Idle as u32, Ordering::Release);
    }

    /// 提交下一次解码循环的定位目标。
    fn sk(&self, pos_ms: u64) {
        self.seek_signal.store(pos_ms, Ordering::Release);
    }

    /// 保存有限的有效音量。
    fn stvlm(&self, vlm: f32) {
        if !vlm.is_finite() { return; }
        let clamped = vlm.clamp(0.0, 1.0);
        self.volume.store(clamped.to_bits(), Ordering::Release);
    }

    /// 从实际消费帧数生成播放快照。
    fn prgrss(&self) -> PlaybackProgress {
        let st_val = self.state.load(Ordering::Acquire);
        let state = match st_val {
            1 => PlaybackState::Resolving,
            2 => PlaybackState::Buffering,
            3 => PlaybackState::Playing,
            4 => PlaybackState::Paused,
            5 => PlaybackState::Seeking,
            6 => PlaybackState::Recovering,
            7 => PlaybackState::Ended,
            8 => PlaybackState::Failed,
            _ => PlaybackState::Idle,
        };

        let output = self.output.lock().unwrap();
        let played_frms = output.as_ref().map(|o| o.plydfrms()).unwrap_or(0);
        let sample_rate = output.as_ref().map(|o| o.config.sample_rate).unwrap_or(44100);
        let pos = self.clock.pos(played_frms, sample_rate);
        let dur = self.total_duration_ms.load(Ordering::Acquire);
        let vol = f32::from_bits(self.volume.load(Ordering::Relaxed));

        PlaybackProgress {
            position_ms: pos,
            duration_ms: dur,
            volume: vol,
            state,
        }
    }
}

/// 以有界缓冲发送交错样本。
fn wrtpcm(ring: &RealtimeRing, samples: &[f32], channels: u16, stop: &AtomicBool, seek: &AtomicU64) {
    let mut offset = 0;
    while offset < samples.len() && !stop.load(Ordering::Acquire) && seek.load(Ordering::Acquire) == u64::MAX {
        let pending = &samples[offset..];
        let (written, _) = ring.psh(pending, (pending.len() / channels as usize) as u64, ring.cap());
        offset += written;
        if written == 0 { std::thread::sleep(std::time::Duration::from_millis(10)); }
    }
}

impl Drop for AudioController {
    /// 退出时回收播放资源。
    fn drop(&mut self) {
        self.stp();
    }
}
