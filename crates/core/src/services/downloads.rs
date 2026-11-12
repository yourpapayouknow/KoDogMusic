use crate::application::ports::{StoragePort, StreamPort};
use crate::deps::{serde::{Deserialize, Serialize}, uuid::Uuid, MediaSource, win::*};
use crate::domain::{track::{AudioQuality, DownloadedFile, TrackSummary}, error::{AppError, StorageError, CatalogError}};
use crate::audio::{source::{FileSource, HttpSource}, decoder::Decoder};
use std::{io::{Read, Write}, path::{Path, PathBuf}, sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}}};
use std::os::windows::ffi::{OsStrExt, OsStringExt};

/// 下载任务状态
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DownloadState {
    Queued,
    Downloading { received: u64, total: Option<u64> },
    Completed,
    Failed(String),
}

/// 下载任务实体
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadJob {
    pub dest_path: String,
    pub state: DownloadState,
    #[serde(default)]
    pub completed: usize,
    #[serde(default)]
    pub total: usize,
}

/// 单任务或批量专辑/歌单下载复用播放源并以有界缓冲分目录写盘。
pub struct DownloadTask {
    pub state: Arc<Mutex<DownloadJob>>,
    cancel: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl DownloadTask {
    /// 启动单曲下载；当前任务结束前不接受另一任务。
    pub fn nw(track: TrackSummary, quality: AudioQuality, directory: PathBuf, stream: Arc<dyn StreamPort>) -> Result<Self, AppError> {
        let collection = track.collname();
        Self::btch(vec![track], collection, quality, directory, stream, None)
    }

    /// 按专辑或歌单单位分目录批量下载并记录本地无损索引。
    pub fn btch(
        tracks: Vec<TrackSummary>,
        collection_name: String,
        quality: AudioQuality,
        directory: PathBuf,
        stream: Arc<dyn StreamPort>,
        storage: Option<Arc<dyn StoragePort>>,
    ) -> Result<Self, AppError> {
        if tracks.is_empty() {
            return Err(CatalogError::NotFound.into());
        }
        let base_dir = if directory.as_os_str().is_empty() { dwnlddir()? } else { directory };
        if !base_dir.is_absolute() {
            return Err(CatalogError::InvalidData("下载目录必须为绝对路径".into()).into());
        }
        let folder_name = flname(&collection_name);
        let target_dir = base_dir.join(&folder_name);
        std::fs::create_dir_all(&target_dir).map_err(ioerr)?;
        let total_tracks = tracks.len();
                let state = Arc::new(Mutex::new(DownloadJob {
            dest_path: String::new(),
            state: DownloadState::Queued,
            completed: 0,
            total: total_tracks,
        }));
        let shared = state.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let stopped = cancel.clone();
        let thread = std::thread::spawn(move || {
            let mut ok_count = 0usize;
            let mut last_path = PathBuf::new();
            let mut last_err: Option<String> = None;
            for (idx, track) in tracks.iter().enumerate() {
                if stopped.load(Ordering::Acquire) {
                    shared.lock().unwrap().state = DownloadState::Failed(AppError::Cancelled.to_string());
                    return;
                }
                shared.lock().unwrap().completed = idx;
                match dwnld(track, quality, &target_dir, stream.as_ref(), &shared, &stopped) {
                    Ok((path, lossless_codec)) => {
                        ok_count += 1;
                        last_path = path.clone();
                        if let Some(ref stg) = storage {
                            let _ = stg.svdwnld(&DownloadedFile {
                                track_id: track.id.clone(),
                                quality,
                                is_lossless: quality.islss() && lossless_codec,
                                collection_name: collection_name.clone(),
                                file_path: path.display().to_string(),
                            });
                        }
                    }
                    Err(err) => {
                        last_err = Some(err.to_string());
                    }
                }
            }
            let mut job = shared.lock().unwrap();
            job.completed = ok_count;
            if ok_count > 0 {
                job.dest_path = if total_tracks == 1 {
                    last_path.display().to_string()
                } else {
                    target_dir.display().to_string()
                };
                job.state = DownloadState::Completed;
            } else {
                job.state = DownloadState::Failed(last_err.unwrap_or_else(|| "下载失败".into()));
            }
        });
        Ok(Self { state, cancel, thread: Some(thread) })
    }

    /// 返回后台下载是否已经结束。
    pub fn done(&self) -> bool { self.thread.as_ref().is_none_or(|thread| thread.is_finished()) }
}

impl Drop for DownloadTask {
    /// 退出时取消写盘并删除未完成临时文件。
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() { let _ = thread.join(); }
    }
}

/// 管理本次下载独有的临时文件。
struct Partial(PathBuf);
impl Drop for Partial {
    /// 清理未发布的文件。
    fn drop(&mut self) { let _ = std::fs::remove_file(&self.0); }
}

/// 下载、核对长度及解码后无覆盖发布文件，并返回目标路径及是否为无损编码。
fn dwnld(
    track: &TrackSummary,
    quality: AudioQuality,
    directory: &Path,
    stream: &dyn StreamPort,
    state: &Mutex<DownloadJob>,
    cancel: &AtomicBool,
) -> Result<(PathBuf, bool), AppError> {
    let source = stream.rslvtrk(track, quality)?;
    let mut reader: Box<dyn MediaSource> = if source.url_or_local.starts_with("http://") || source.url_or_local.starts_with("https://") {
        Box::new(HttpSource::nw(&source.url_or_local)?)
    } else { Box::new(FileSource::opn(&source.url_or_local)?) };
    let total = reader.byte_len();
    let maximum = 2 * 1024 * 1024 * 1024u64;
    if total.is_some_and(|total| total > maximum) { return Err(CatalogError::InvalidData("音频文件超过下载大小限制".into()).into()); }
    let partial = Partial(directory.join(format!(".kodog-{}.part", Uuid::new_v4())));
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&partial.0).map_err(ioerr)?;
    let mut buffer = [0u8; 64 * 1024];
    let mut received = 0u64;
    loop {
        if cancel.load(Ordering::Acquire) { return Err(AppError::Cancelled); }
        let count = reader.read(&mut buffer).map_err(ioerr)?;
        if count == 0 { break; }
        received += count as u64;
        if received > maximum { return Err(CatalogError::InvalidData("音频文件超过下载大小限制".into()).into()); }
        file.write_all(&buffer[..count]).map_err(ioerr)?;
        state.lock().unwrap().state = DownloadState::Downloading { received, total };
    }
    if received == 0 || total.is_some_and(|total| total != received) { return Err(CatalogError::InvalidData("下载文件长度不完整".into()).into()); }
    file.sync_all().map_err(ioerr)?;
    drop(file);
    let mut decoder = Decoder::opnfil(&partial.0)?;
    if !decoder.nxtchk(&mut Vec::new())? { return Err(CatalogError::InvalidData("下载内容没有音频数据".into()).into()); }
    drop(decoder);
    let hint = source.codec_hint.as_deref().or_else(|| Path::new(&source.url_or_local).extension().and_then(|value| value.to_str())).unwrap_or("audio").to_ascii_lowercase();
    let ext = match hint.as_str() { "mp3" | "flac" | "wav" | "aac" | "m4a" | "ogg" => hint.as_str(), _ => "audio" };
    let lossless_codec = matches!(ext, "flac" | "wav");
    let base = track.colttl();
    let base = flname(&base);
    for suffix in 0..10_000 {
        if cancel.load(Ordering::Acquire) { return Err(AppError::Cancelled); }
        let filename = if suffix == 0 { format!("{base}.{ext}") } else { format!("{base} ({suffix}).{ext}") };
        let dest = directory.join(filename);
        let from: Vec<u16> = partial.0.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<u16> = dest.as_os_str().encode_wide().chain(Some(0)).collect();
        if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), MOVEFILE_WRITE_THROUGH) } != 0 { return Ok((dest, lossless_codec)); }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::AlreadyExists { return Err(ioerr(error)); }
    }
    Err(CatalogError::InvalidData("同名下载文件过多".into()).into())
}

/// 读取 Windows 已知的下载目录。
pub fn dwnlddir() -> Result<PathBuf, AppError> {
    let mut pointer = std::ptr::null_mut();
    if unsafe { SHGetKnownFolderPath(&FOLDERID_Downloads, 0, std::ptr::null_mut(), &mut pointer) } < 0 || pointer.is_null() {
        return Err(StorageError::Permission.into());
    }
    let mut length = 0;
    unsafe { while *pointer.add(length) != 0 { length += 1; } }
    let name = unsafe { std::ffi::OsString::from_wide(std::slice::from_raw_parts(pointer, length)) };
    unsafe { CoTaskMemFree(pointer.cast()); }
    Ok(PathBuf::from(name).join("KoDogLite"))
}

/// 清除 Windows 文件名中的保留字符和设备名。
pub fn flname(name: &str) -> String {
    let clean: String = name.chars().take(100).map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { '_' } else { c }).collect();
    let clean = clean.trim().trim_end_matches(['.', ' ']);
    let stem = clean.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL") || (stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && matches!(stem.as_bytes()[3], b'1'..=b'9'));
    if clean.is_empty() { "track".into() } else if reserved { format!("_{clean}") } else { clean.to_string() }
}

/// 将磁盘错误纳入统一错误类型。
fn ioerr(error: std::io::Error) -> AppError { StorageError::Io(error.to_string()).into() }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ids::TrackId;
    /// 验证文件名、无覆盖发布、错误内容与临时文件清理。
    #[test]
    fn tst_dwnld() {
        struct Local(PathBuf);
        impl StreamPort for Local {
            /// 以测试生成的本地 WAV 文件作为播放源。
            fn rslv(&self, _: &TrackId, _: AudioQuality) -> Result<crate::domain::track::ResolvedSource, AppError> {
                Ok(crate::domain::track::ResolvedSource { url_or_local: self.0.display().to_string(), codec_hint: Some("wav".into()) })
            }
        }
        assert_eq!(flname(" ../a:b? "), ".._a_b_");
        assert_eq!(flname("CON.txt"), "_CON.txt");
        let root = std::env::temp_dir().join(format!("kodog-download-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("source.wav");
        let mut wave = b"RIFF".to_vec(); wave.extend(44u32.to_le_bytes()); wave.extend(b"WAVEfmt "); wave.extend(16u32.to_le_bytes());
        wave.extend(1u16.to_le_bytes()); wave.extend(1u16.to_le_bytes()); wave.extend(8000u32.to_le_bytes()); wave.extend(16000u32.to_le_bytes());
        wave.extend(2u16.to_le_bytes()); wave.extend(16u16.to_le_bytes()); wave.extend(b"data"); wave.extend(8u32.to_le_bytes()); wave.extend([0u8; 8]);
        std::fs::write(&source, &wave).unwrap();
        let track = TrackSummary::nw(TrackId::lcl(source.display().to_string()), "sample", "");
        let state = Mutex::new(DownloadJob {
            dest_path: String::new(), state: DownloadState::Queued,
            completed: 0, total: 1,
        });
        let output = root.join("out"); std::fs::create_dir(&output).unwrap();
        let cancel = AtomicBool::new(false);
        let (first, l1) = dwnld(&track, AudioQuality::High, &output, &Local(source.clone()), &state, &cancel).unwrap();
        let (second, _) = dwnld(&track, AudioQuality::High, &output, &Local(source.clone()), &state, &cancel).unwrap();
        assert!(l1);
        assert_ne!(first, second); assert_eq!(std::fs::read(first).unwrap(), wave);
        let stg = std::sync::Arc::new(crate::storage::database::DatabaseStorage::inmem().unwrap());
        let batch = DownloadTask::btch(
            vec![track.clone()],
            "测试专辑".into(),
            AudioQuality::Lossless,
            output.clone(),
            std::sync::Arc::new(Local(source.clone())),
            Some(stg.clone()),
        ).unwrap();
        while !batch.done() { std::thread::sleep(std::time::Duration::from_millis(10)); }
        assert!(output.join("测试专辑").join("sample.wav").is_file());
        assert!(crate::application::ports::StoragePort::lddwnld(stg.as_ref(), &track.id).unwrap().is_some_and(|d| d.is_lossless));
        drop(batch);
        std::fs::write(&source, "<html>error</html>").unwrap();
        assert!(dwnld(&track, AudioQuality::High, &output, &Local(source), &state, &cancel).is_err());
        assert_eq!(std::fs::read_dir(&output).unwrap().count(), 3);
        std::fs::remove_dir_all(root).unwrap();
    }
}
