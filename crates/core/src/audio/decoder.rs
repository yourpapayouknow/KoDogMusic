use std::path::Path;
use crate::deps::*;
use crate::domain::error::{AppError, PlaybackError};

/// 音频流参数
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamSpec {
    pub sample_rate: u32,
    pub channels: u16,
}

/// Symphonia 流式音频解码器
pub struct Decoder {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    spec: StreamSpec,
    total_frames: Option<u64>,
    time_base: Option<TimeBase>,
    base_secs: f64,
}

impl Decoder {
    /// 打开本地音频文件
    pub fn opnfil(path: impl AsRef<Path>) -> Result<Self, AppError> {
        let file = std::fs::File::open(path.as_ref())
            .map_err(|e| PlaybackError::Decode(e.to_string()))?;
        let mss = MediaSourceStream::new(Box::new(file), MediaSourceStreamOptions::default());

        let mut hint = Hint::new();
        if let Some(ext) = path.as_ref().extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }

        Self::frmmss(mss, &hint)
    }

    /// 从媒体源流构造解码器
    pub fn frmmss(mss: MediaSourceStream<'static>, hint: &Hint) -> Result<Self, AppError> {
        let format = get_probe()
            .probe(
                hint,
                mss,
                FormatOptions::default(),
                MetadataOptions::default(),
            )
            .map_err(|_e| PlaybackError::UnsupportedFormat)?;

        Self::frmfmt(format)
    }

    /// 从格式读取器构造解码器
    fn frmfmt(format: Box<dyn FormatReader>) -> Result<Self, AppError> {
        let track = format
            .default_track(TrackType::Audio)
            .ok_or(PlaybackError::UnsupportedFormat)?;

        let track_id = track.id;
        let params = match track.codec_params.as_ref() {
            Some(CodecParameters::Audio(p)) => p.clone(),
            _ => return Err(PlaybackError::UnsupportedFormat.into()),
        };

        let spec = StreamSpec {
            sample_rate: params.sample_rate.unwrap_or(44100),
            channels: params
                .channels
                .as_ref()
                .map(|c| c.count() as u16)
                .unwrap_or(2),
        };

        let total_frames = track.num_frames;
        let time_base = track.time_base;

        let decoder = get_codecs()
            .make_audio_decoder(&params, &AudioDecoderOptions::default())
            .map_err(|e| PlaybackError::Decode(e.to_string()))?;

        Ok(Self {
            format,
            decoder,
            track_id,
            spec,
            total_frames,
            time_base,
            base_secs: 0.0,
        })
    }

    /// 读取流规格
    pub fn spec(&self) -> StreamSpec {
        self.spec
    }

    /// 计算总时长（秒）
    pub fn totdursec(&self) -> Option<f64> {
        self.total_frames.map(|f| {
            if self.spec.sample_rate == 0 {
                0.0
            } else {
                f as f64 / self.spec.sample_rate as f64
            }
        })
    }

    /// 解码下一块 PCM 样本到复用缓冲，返回是否读到音频块。
    pub fn nxtchk(&mut self, out: &mut Vec<f32>) -> Result<bool, AppError> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(Some(p)) => p,
                Ok(None) => return Ok(false),
                // 流尾不完整按结束处理。
                Err(SymphoniaError::IoError(e))
                    if e.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    return Ok(false);
                }
                Err(e) => return Err(PlaybackError::Decode(e.to_string()).into()),
            };

            if packet.track_id != self.track_id {
                continue;
            }

            let decoded: GenericAudioBufferRef<'_> = match self.decoder.decode(&packet) {
                Ok(d) => d,
                Err(SymphoniaError::DecodeError(_)) => continue,
                Err(SymphoniaError::IoError(e))
                    if e.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    return Ok(false);
                }
                Err(SymphoniaError::ResetRequired) => {
                    return Err(PlaybackError::Decode("解码器要求重置".into()).into());
                }
                Err(e) => return Err(PlaybackError::Decode(e.to_string()).into()),
            };

            if decoded.is_empty() {
                continue;
            }

            let actual = decoded.spec();
            let actual_spec = StreamSpec {
                sample_rate: actual.rate(),
                channels: actual.channels().count() as u16,
            };
            if actual_spec != self.spec {
                self.spec = actual_spec;
            }

            out.clear();
            decoded.copy_to_vec_interleaved(out);

            return Ok(true);
        }
    }

    /// 定位到指定秒数位置
    pub fn sk(&mut self, pos_secs: f64) -> Result<f64, AppError> {
        let pos = pos_secs.max(0.0);
        let secs = pos.trunc() as i64;
        let nanos = (pos.fract() * 1_000_000_000.0).round() as u32;
        let (secs, nanos) = if nanos >= 1_000_000_000 {
            (secs + 1, 0)
        } else {
            (secs, nanos)
        };
        let time = Time::try_new(secs, nanos).unwrap_or(Time::ZERO);

        let seeked = self
            .format
            .seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time,
                    track_id: Some(self.track_id),
                },
            )
            .map_err(|e| PlaybackError::Decode(e.to_string()))?;

        self.decoder.reset();

        let landed = match self.time_base {
            Some(tb) => tb.calc_time_saturating(seeked.actual_ts).as_secs_f64(),
            None => pos,
        };

        self.base_secs = landed.max(0.0);
        Ok(self.base_secs)
    }
}
