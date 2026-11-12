use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use crate::deps::reqwest::blocking::Client;
use crate::deps::symphonia::core::io::MediaSource;
use crate::domain::error::{AppError, PlaybackError};

const WIN_CAP: usize = 4 * 1024 * 1024;
const CHUNK: u64 = 512 * 1024;

/// 本地文件音频源
pub struct FileSource {
    file: File,
    len: u64,
}

impl FileSource {
    /// 打开本地音频文件
    pub fn opn(path: impl AsRef<Path>) -> Result<Self, AppError> {
        let file = File::open(path.as_ref())
            .map_err(|e| PlaybackError::Decode(e.to_string()))?;
        let len = file.metadata().map(|m| m.len()).unwrap_or(0);
        Ok(Self { file, len })
    }
}

impl Read for FileSource {
    /// 读取本地文件字节。
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.file.read(buf)
    }
}

impl Seek for FileSource {
    /// 定位本地文件。
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.file.seek(pos)
    }
}

impl MediaSource for FileSource {
    /// 报告本地文件可定位。
    fn is_seekable(&self) -> bool {
        true
    }

    /// 返回本地文件长度。
    fn byte_len(&self) -> Option<u64> {
        Some(self.len)
    }
}

/// HTTP Range 网络音频流数据源
pub struct HttpSource {
    client: Client,
    url: String,
    total: Option<u64>,
    win: Vec<u8>,
    base: u64,
    pos: u64,
    filled: u64,
}

impl HttpSource {
    /// 打开网络流媒体
    pub fn nw(url: &str) -> Result<Self, AppError> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(8))
            .build()
            .map_err(|e| PlaybackError::Decode(e.to_string()))?;

        let mut s = Self {
            client,
            url: url.to_string(),
            total: None,
            win: Vec::with_capacity(WIN_CAP),
            base: 0,
            pos: 0,
            filled: 0,
        };
        s.fill()?;
        Ok(s)
    }

    /// 分段拉取网络流
    fn fill(&mut self) -> Result<(), AppError> {
        let end = match self.total {
            Some(t) => self.pos.saturating_add(CHUNK - 1).min(t.saturating_sub(1)),
            None => self.pos.saturating_add(CHUNK - 1),
        };

        let mut resp = self
            .client
            .get(&self.url)
            .header("Range", format!("bytes={}-{}", self.pos, end))
            .send()
            .map_err(|e| PlaybackError::Decode(e.without_url().to_string()))?;

        if !resp.status().is_success() {
            return Err(PlaybackError::Decode(format!("HTTP 状态码: {}", resp.status())).into());
        }

        if self.pos > 0 && resp.status() != crate::deps::reqwest::StatusCode::PARTIAL_CONTENT {
            return Err(PlaybackError::Decode("音源不支持分段定位".into()).into());
        }

        if let Some(cr) = resp.headers().get("content-range").and_then(|v| v.to_str().ok()) {
            if let Some((_, total)) = cr.rsplit_once('/') {
                if let Ok(t) = total.trim().parse::<u64>() {
                    self.total = Some(t);
                }
            }
        }

        if self.total.is_none() {
            self.total = resp.content_length();
        }

        // 每次只读取一个有界窗口。
        let mut bytes = Vec::with_capacity(CHUNK as usize);
        Read::by_ref(&mut resp).take(CHUNK).read_to_end(&mut bytes)
            .map_err(|e| PlaybackError::Decode(e.to_string()))?;
        self.base = self.pos;
        self.win = bytes;
        self.filled = self.base + self.win.len() as u64;
        Ok(())
    }
}

impl Read for HttpSource {
    /// 读取网络流字节，耗尽窗口时拉取下一段。
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if let Some(total) = self.total {
            if self.pos >= total {
                return Ok(0);
            }
        }

        if self.pos < self.base || self.pos >= self.filled {
            if let Err(e) = self.fill() {
                return Err(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()));
            }
        }

        let offset = (self.pos - self.base) as usize;
        if offset >= self.win.len() {
            return Ok(0);
        }

        let avail = &self.win[offset..];
        let n = buf.len().min(avail.len());
        buf[..n].copy_from_slice(&avail[..n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for HttpSource {
    /// 定位网络流读取位置。
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let target = match pos {
            SeekFrom::Start(n) => n,
            SeekFrom::Current(n) => {
                if n >= 0 {
                    self.pos.saturating_add(n as u64)
                } else {
                    self.pos.saturating_sub((-n) as u64)
                }
            }
            SeekFrom::End(n) => match self.total {
                Some(t) => {
                    if n >= 0 {
                        t.saturating_add(n as u64)
                    } else {
                        t.saturating_sub((-n) as u64)
                    }
                }
                None => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "未知总长度流不支持从尾部寻道",
                    ))
                }
            },
        };

        self.pos = target;
        Ok(self.pos)
    }
}

impl MediaSource for HttpSource {
    /// 报告网络流支持分段定位。
    fn is_seekable(&self) -> bool {
        true
    }

    /// 返回网络流总长度。
    fn byte_len(&self) -> Option<u64> {
        self.total
    }
}
