use crate::audio::output::OutConfig;
use crate::deps::*;
use crate::domain::error::{AppError, PlaybackError};

const CHUNK: usize = 1024;
/// 音频样本转换器
pub struct Conv {
    src_rate: u32,
    src_ch: u16,
    dst_rate: u32,
    dst_ch: u16,
    rsmp: Option<Fft<f32>>,
    inbuf: Vec<f32>,
    outbuf: Vec<f32>,
    pending: Vec<f32>,
    delay_left: usize,
    total_in: usize,
    emit_frames: usize,
    flushed: bool,
}

impl Conv {
    /// 构造转换器
    pub fn nw(src_rate: u32, src_ch: u16, cfg: &OutConfig) -> Result<Self, AppError> {
        if src_rate == 0 || src_ch == 0 || cfg.sample_rate == 0 || cfg.channels == 0 {
            return Err(PlaybackError::UnsupportedFormat.into());
        }
        let need_rsmp = src_rate != cfg.sample_rate;

        let (rsmp, inbuf, outbuf, delay) = if need_rsmp {
            let r = Fft::new(
                src_rate as usize,
                cfg.sample_rate as usize,
                CHUNK,
                src_ch as usize,
                FixedSync::Both,
            )
            .map_err(|e| PlaybackError::Decode(e.to_string()))?;

            let in_frames = r.input_frames_next();
            let out_frames = r.output_frames_max();
            let delay = r.output_delay();
            (
                Some(r),
                vec![0.0f32; in_frames * src_ch as usize],
                vec![0.0f32; out_frames * src_ch as usize],
                delay,
            )
        } else {
            (None, Vec::new(), Vec::new(), 0)
        };

        Ok(Self {
            src_rate,
            src_ch,
            dst_rate: cfg.sample_rate,
            dst_ch: cfg.channels,
            rsmp,
            inbuf,
            outbuf,
            pending: Vec::new(),
            delay_left: delay,
            total_in: 0,
            emit_frames: 0,
            flushed: false,
        })
    }

    /// 转换一批交错 PCM 样本，结果追加至 out
    pub fn cnvr(&mut self, src: &[f32], out: &mut Vec<f32>) -> Result<(), AppError> {
        if src.is_empty() {
            return Ok(());
        }
        let src_ch = self.src_ch as usize;
        self.total_in += src.len() / src_ch;

        if self.rsmp.is_some() {
            self.pending.extend_from_slice(src);
            let in_frames = self.rsmp.as_ref().unwrap().input_frames_next();
            let chunk_samples = in_frames * src_ch;

            while self.pending.len() >= chunk_samples {
                self.inbuf.copy_from_slice(&self.pending[..chunk_samples]);
                self.pending.drain(..chunk_samples);

                let in_slice = InterleavedSlice::new(&self.inbuf, src_ch, in_frames)
                    .map_err(|e| PlaybackError::Decode(e.to_string()))?;
                let max_out_frames = self.outbuf.len() / src_ch;
                let mut out_slice = InterleavedSlice::new_mut(&mut self.outbuf, src_ch, max_out_frames)
                    .map_err(|e| PlaybackError::Decode(e.to_string()))?;

                let (_, out_frames) = self.rsmp.as_mut().unwrap()
                    .process_into_buffer(&in_slice, &mut out_slice, None)
                    .map_err(|e| PlaybackError::Decode(e.to_string()))?;

                self.emtfrm(out_frames, out);
            }
        } else {
            Self::chnlmap(self.src_ch, self.dst_ch, src, out);
        }
        Ok(())
    }

    /// 排出重采样尾部并修剪滤波延迟。
    pub fn flsh(&mut self, out: &mut Vec<f32>) -> Result<(), AppError> {
        if self.flushed || self.rsmp.is_none() { return Ok(()); }
        self.flushed = true;
        let channels = self.src_ch as usize;
        let input_frames = self.inbuf.len() / channels;
        let output_frames = self.outbuf.len() / channels;
        let target = self.total_in * self.dst_rate as usize / self.src_rate as usize;
        let mut partial = self.pending.len() / channels;
        self.inbuf.fill(0.0);
        self.inbuf[..self.pending.len()].copy_from_slice(&self.pending);
        self.pending.clear();
        let mut idle = 0u8;
        while self.emit_frames < target {
            let input = InterleavedSlice::new(&self.inbuf, channels, input_frames)
                .map_err(|e| PlaybackError::Decode(e.to_string()))?;
            let mut output = InterleavedSlice::new_mut(&mut self.outbuf, channels, output_frames)
                .map_err(|e| PlaybackError::Decode(e.to_string()))?;
            let indexing = Indexing::new().partial_len(partial);
            let (_, frames) = self.rsmp.as_mut().unwrap().process_into_buffer(&input, &mut output, Some(&indexing))
                .map_err(|e| PlaybackError::Decode(e.to_string()))?;
            self.emtfrm(frames, out);
            self.inbuf.fill(0.0);
            partial = 0;
            // 无新帧时结束冲刷。
            if frames == 0 {
                idle = idle.saturating_add(1);
                if idle >= 4 { break; }
            } else {
                idle = 0;
            }
        }
        Ok(())
    }

    /// 输出可用帧并保持原始时长。
    fn emtfrm(&mut self, frames: usize, out: &mut Vec<f32>) {
        let skip = self.delay_left.min(frames);
        self.delay_left -= skip;
        let target = self.total_in * self.dst_rate as usize / self.src_rate as usize;
        let take = (frames - skip).min(target.saturating_sub(self.emit_frames));
        let channels = self.src_ch as usize;
        Self::chnlmap(self.src_ch, self.dst_ch, &self.outbuf[skip * channels..(skip + take) * channels], out);
        self.emit_frames += take;
    }

    /// 声道布局适配映射
    fn chnlmap(src_ch: u16, dst_ch: u16, src: &[f32], out: &mut Vec<f32>) {
        let src_ch = src_ch as usize;
        let dst_ch = dst_ch as usize;

        if src_ch == dst_ch {
            out.extend_from_slice(src);
        } else if src_ch == 1 && dst_ch == 2 {
            for &s in src {
                out.push(s);
                out.push(s);
            }
        } else if src_ch == 2 && dst_ch == 1 {
            for chunk in src.chunks_exact(2) {
                out.push((chunk[0] + chunk[1]) * 0.5);
            }
        } else {
            // 通用通道截断或补零
            for chunk in src.chunks(src_ch) {
                for ch in 0..dst_ch {
                    if ch < chunk.len() {
                        out.push(chunk[ch]);
                    } else {
                        out.push(0.0);
                    }
                }
            }
        }
    }
}
