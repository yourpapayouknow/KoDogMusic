use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use crate::audio::ring::RealtimeRing;
use crate::deps::cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crate::deps::cpal::{
    default_host, BufferSize, Device, FromSample, SampleFormat, SizedSample, Stream, StreamConfig,
};
use crate::domain::error::{AppError, PlaybackError};

const CHUNK: usize = 1024;

/// 输出设备的采样格式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutFormat {
    I16,
    I32,
    F32,
    F64,
    U8,
    U16,
}

impl OutFormat {
    /// 由 cpal 采样格式转换
    pub fn frmcpal(f: SampleFormat) -> Result<Self, AppError> {
        match f {
            SampleFormat::I16 => Ok(OutFormat::I16),
            SampleFormat::I32 => Ok(OutFormat::I32),
            SampleFormat::F32 => Ok(OutFormat::F32),
            SampleFormat::F64 => Ok(OutFormat::F64),
            SampleFormat::U8 => Ok(OutFormat::U8),
            SampleFormat::U16 => Ok(OutFormat::U16),
            other => Err(PlaybackError::Device(format!("不支持的采样格式: {other:?}")).into()),
        }
    }
}

/// 输出流配置
#[derive(Debug, Clone, Copy)]
pub struct OutConfig {
    pub sample_rate: u32,
    pub channels: u16,
    pub format: OutFormat,
}

/// 音频硬件输出流封装
pub struct Output {
    stream: Stream,
    pub config: OutConfig,
    pub(crate) played_frames: Arc<AtomicU64>,
}

impl Output {
    /// 获取默认输出设备的配置
    pub fn defcfg() -> Result<OutConfig, AppError> {
        let dev = default_host()
            .default_output_device()
            .ok_or_else(|| PlaybackError::Device("未找到默认音频输出设备".into()))?;
        let sup = dev
            .default_output_config()
            .map_err(|e| PlaybackError::Device(e.to_string()))?;
        Ok(OutConfig {
            sample_rate: sup.sample_rate(),
            channels: sup.channels(),
            format: OutFormat::frmcpal(sup.sample_format())?,
        })
    }

    /// 使用默认设备创建音频输出流
    pub fn nw(ring: Arc<RealtimeRing>, config: OutConfig, volume: Arc<AtomicU32>) -> Result<Self, AppError> {
        let dev = default_host()
            .default_output_device()
            .ok_or_else(|| PlaybackError::Device("未找到默认音频输出设备".into()))?;
        Self::ondev(&dev, ring, config, volume)
    }

    /// 在指定设备上创建音频输出流
    pub fn ondev(dev: &Device, ring: Arc<RealtimeRing>, config: OutConfig, volume: Arc<AtomicU32>) -> Result<Self, AppError> {
        let played = Arc::new(AtomicU64::new(0));
        let sc = StreamConfig {
            channels: config.channels,
            sample_rate: config.sample_rate,
            buffer_size: BufferSize::Default,
        };

        let stream = match config.format {
            OutFormat::F32 => Self::mkstrm::<f32>(dev, &sc, ring, played.clone(), volume)?,
            OutFormat::I16 => Self::mkstrm::<i16>(dev, &sc, ring, played.clone(), volume)?,
            OutFormat::I32 => Self::mkstrm::<i32>(dev, &sc, ring, played.clone(), volume)?,
            OutFormat::F64 => Self::mkstrm::<f64>(dev, &sc, ring, played.clone(), volume)?,
            OutFormat::U8 => Self::mkstrm::<u8>(dev, &sc, ring, played.clone(), volume)?,
            OutFormat::U16 => Self::mkstrm::<u16>(dev, &sc, ring, played.clone(), volume)?,
        };

        Ok(Self {
            stream,
            config,
            played_frames: played,
        })
    }

    /// 构建具体样本类型的输出流
    fn mkstrm<T>(
        dev: &Device,
        sc: &StreamConfig,
        ring: Arc<RealtimeRing>,
        played: Arc<AtomicU64>,
        volume: Arc<AtomicU32>,
    ) -> Result<Stream, AppError>
    where
        T: SizedSample + FromSample<f32>,
    {
        let channels = sc.channels as usize;
        let mut scratch = vec![0.0f32; CHUNK];

        let stream = dev
            .build_output_stream(
                *sc,
                move |out: &mut [T], _info| {
                    // 清零欠载并应用音量。
                    out.fill(T::from_sample_(0.0));
                    let gain = f32::from_bits(volume.load(Ordering::Relaxed));
                    let mut written = 0usize;
                    while written < out.len() {
                        let want = (out.len() - written).min(scratch.len());
                        let (n, _) = ring.pop(&mut scratch[..want]);
                        if n == 0 {
                            break;
                        }
                        for i in 0..n {
                            out[written + i] = T::from_sample_(scratch[i] * gain);
                        }
                        written += n;
                    }
                    played.fetch_add((written / channels) as u64, Ordering::Relaxed);
                },
                |e| crate::deps::tracing::error!(error = %e, "音频输出设备异常"),
                None,
            )
            .map_err(|e| PlaybackError::Device(e.to_string()))?;

        Ok(stream)
    }

    /// 启动硬件流播放
    pub fn ply(&self) -> Result<(), AppError> {
        self.stream
            .play()
            .map_err(|e| PlaybackError::Device(e.to_string()).into())
    }

    /// 暂停硬件流播放
    pub fn ps(&self) -> Result<(), AppError> {
        self.stream
            .pause()
            .map_err(|e| PlaybackError::Device(e.to_string()).into())
    }

    /// 读取已播放帧数
    pub fn plydfrms(&self) -> u64 {
        self.played_frames.load(Ordering::Relaxed)
    }
}
