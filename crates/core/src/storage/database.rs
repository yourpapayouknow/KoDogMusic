use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::io::Write;
use std::sync::{Arc, Mutex};
use crate::application::ports::StoragePort;
use crate::deps::rusqlite::{Connection, OptionalExtension, Row};
use crate::domain::error::{AppError, StorageError};
use crate::domain::ids::{ProviderId, TrackId};
use crate::domain::settings::AppSettings;
use crate::domain::track::{clnttl, AudioQuality, CoverDeck, DeckKind, DownloadedFile, TrackSummary};
use crate::storage::migrations::mgtr;

/// SQLite 本地存储引擎
pub struct DatabaseStorage {
    conn: Arc<Mutex<Connection>>,
    config_path: PathBuf,
    memory_cfg: Mutex<Option<AppSettings>>,
}

impl DatabaseStorage {
    /// 打开本地数据库并执行迁移
    pub fn opn(db_path: impl AsRef<Path>, config_path: impl AsRef<Path>) -> Result<Self, AppError> {
        let p = db_path.as_ref();
        if let Some(parent) = p.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|_| StorageError::Permission)?;
            }
        }
        let conn = Connection::open(p)
            .map_err(|e| StorageError::Migration(e.to_string()))?;
        mgtr(&conn)?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            config_path: config_path.as_ref().to_path_buf(),
            memory_cfg: Mutex::new(None),
        })
    }

    /// 创建纯内存数据库（用于测试）
    pub fn inmem() -> Result<Self, AppError> {
        let conn = Connection::open_in_memory()
            .map_err(|e| StorageError::Migration(e.to_string()))?;
        mgtr(&conn)?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            config_path: PathBuf::new(),
            memory_cfg: Mutex::new(Some(AppSettings::default())),
        })
    }
}

impl StoragePort for DatabaseStorage {
    /// 读取完整配置并验证音量。
    fn ldcfg(&self) -> Result<AppSettings, AppError> {
        if let Some(cfg) = self.memory_cfg.lock().map_err(|_| StorageError::Corrupt)?.as_ref() {
            return Ok(cfg.clone());
        }
        if !self.config_path.exists() {
            return Ok(AppSettings::default());
        }
        let txt = std::fs::read_to_string(&self.config_path)
            .map_err(|_| StorageError::Permission)?;
        let cfg: AppSettings = crate::deps::toml::from_str(&txt)
            .map_err(|_| StorageError::Corrupt)?;
        vldcfg(&cfg)?;
        Ok(cfg)
    }

    /// 原子保存配置或更新内存配置。
    fn svcfg(&self, cfg: &AppSettings) -> Result<(), AppError> {
        vldcfg(cfg)?;
        {
            let mut memory = self.memory_cfg.lock().map_err(|_| StorageError::Corrupt)?;
            if memory.is_some() {
                *memory = Some(cfg.clone());
                return Ok(());
            }
        }
        let txt = crate::deps::toml::to_string_pretty(cfg)
            .map_err(|e| StorageError::Migration(e.to_string()))?;
        wrtatom(&self.config_path, txt.as_bytes())
    }

    /// 以事务保存队列和游标。
    fn svqu(&self, trks: &[TrackId], crr: Option<usize>) -> Result<(), AppError> {
        if crr.is_some_and(|index| index >= trks.len()) {
            return Err(StorageError::Corrupt.into());
        }
        let mut conn = self.conn.lock().map_err(|_| StorageError::Corrupt)?;
        let tx = conn.transaction()
            .map_err(|e| StorageError::Migration(e.to_string()))?;
        tx.execute("DELETE FROM queue_items", [])
            .map_err(|e| StorageError::Migration(e.to_string()))?;

        for (pos, id) in trks.iter().enumerate() {
            tx.execute(
                "INSERT INTO queue_items (pos, provider, track_id) VALUES (?1, ?2, ?3)",
                crate::deps::rusqlite::params![pos as i64, prvstr(id), id.raw_id],
            )
            .map_err(|e| StorageError::Migration(e.to_string()))?;
        }

        tx.execute(
            "INSERT INTO playback_state (id, cursor_pos) VALUES (1, ?1) ON CONFLICT(id) DO UPDATE SET cursor_pos = excluded.cursor_pos",
            crate::deps::rusqlite::params![crr.map(|c| c as i64)],
        )
        .map_err(|e| StorageError::Migration(e.to_string()))?;

        tx.commit().map_err(|e| StorageError::Migration(e.to_string()))?;
        Ok(())
    }

    /// 读取完整队列并验证游标。
    fn ldqu(&self) -> Result<(Vec<TrackId>, Option<usize>), AppError> {
        let conn = self.conn.lock().map_err(|_| StorageError::Corrupt)?;
        let mut stmt = conn
            .prepare("SELECT provider, track_id FROM queue_items ORDER BY pos ASC")
            .map_err(|e| StorageError::Migration(e.to_string()))?;

        let rows = stmt
            .query_map([], |row| {
                let p_str: String = row.get(0)?;
                let raw_id: String = row.get(1)?;
                let provider = prvof(&p_str).ok_or(crate::deps::rusqlite::Error::InvalidQuery)?;
                Ok(TrackId { provider, raw_id })
            })
            .map_err(|e| StorageError::Migration(e.to_string()))?;

        let items = rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| StorageError::Migration(e.to_string()))?;

        let cursor: Option<i64> = conn
            .query_row(
                "SELECT cursor_pos FROM playback_state WHERE id = 1",
                [],
                |row| {
                    row.get(0)
                },
            )
            .optional().map_err(|e| StorageError::Migration(e.to_string()))?.flatten();
        let cursor = match cursor {
            Some(value) if value >= 0 && (value as usize) < items.len() => Some(value as usize),
            Some(_) => return Err(StorageError::Corrupt.into()),
            None => None,
        };

        Ok((items, cursor))
    }

    /// 批量写入曲目元数据。
    fn svtrks(&self, tracks: &[TrackSummary]) -> Result<(), AppError> {
        let mut conn = self.conn.lock().map_err(|_| StorageError::Corrupt)?;
        let tx = conn.transaction().map_err(|e| StorageError::Migration(e.to_string()))?;
        for track in tracks {
            let hashes = if track.extra_hashes.is_empty() {
                None
            } else {
                crate::deps::serde_json::to_string(&track.extra_hashes).ok()
            };
            tx.execute(
                "INSERT INTO tracks (provider, track_id, title, artist_text, album_text, duration_ms, cover_key, album_id, album_name, album_audio_id, extra_hashes, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, unixepoch()) ON CONFLICT(provider, track_id) DO UPDATE SET title=excluded.title, artist_text=excluded.artist_text, album_text=excluded.album_text, duration_ms=excluded.duration_ms, cover_key=excluded.cover_key, album_id=COALESCE(excluded.album_id, tracks.album_id), album_name=COALESCE(excluded.album_name, tracks.album_name), album_audio_id=COALESCE(excluded.album_audio_id, tracks.album_audio_id), extra_hashes=COALESCE(excluded.extra_hashes, tracks.extra_hashes), updated_at=excluded.updated_at",
                crate::deps::rusqlite::params![
                    prvstr(&track.id),
                    track.id.raw_id,
                    clnttl(&track.title),
                    track.artist_text,
                    track.album_name,
                    track.duration_ms.and_then(|v| i64::try_from(v).ok()),
                    track.cover_key,
                    track.album_id,
                    track.album_name,
                    track.album_audio_id,
                    hashes,
                ],
            ).map_err(|e| StorageError::Migration(e.to_string()))?;
        }
        tx.commit().map_err(|e| StorageError::Migration(e.to_string()))?;
        Ok(())
    }

    /// 查询单首曲目的元数据。
    fn ldtrk(&self, id: &TrackId) -> Result<Option<TrackSummary>, AppError> {
        let conn = self.conn.lock().map_err(|_| StorageError::Corrupt)?;
        conn.query_row(
            "SELECT provider, track_id, title, artist_text, cover_key, duration_ms, album_id, album_name, album_audio_id, extra_hashes FROM tracks WHERE provider=?1 AND track_id=?2",
            crate::deps::rusqlite::params![prvstr(id), id.raw_id], rdtrk,
        ).optional().map_err(|e| StorageError::Migration(e.to_string()).into())
    }

    /// 读取有元数据的本地收藏。
    fn ldfvrt(&self) -> Result<Vec<TrackSummary>, AppError> {
        let conn = self.conn.lock().map_err(|_| StorageError::Corrupt)?;
        let mut stmt = conn.prepare(
            "SELECT t.provider, t.track_id, t.title, t.artist_text, t.cover_key, t.duration_ms, t.album_id, t.album_name, t.album_audio_id, t.extra_hashes FROM favorites f JOIN tracks t ON t.provider=f.provider AND t.track_id=f.track_id WHERE f.sync_state='local' ORDER BY f.updated_at DESC, f.rowid DESC",
        ).map_err(|e| StorageError::Migration(e.to_string()))?;
        stmt.query_map([], rdtrk).map_err(|e| StorageError::Migration(e.to_string()))?
            .collect::<Result<Vec<_>, _>>().map_err(|e| StorageError::Migration(e.to_string()).into())
    }

    /// 设置本地收藏标记。
    fn stfvrt(&self, id: &TrackId, enabled: bool) -> Result<(), AppError> {
        let conn = self.conn.lock().map_err(|_| StorageError::Corrupt)?;
        if enabled {
            conn.execute(
                "INSERT INTO favorites (provider, track_id, sync_state, updated_at) VALUES (?1, ?2, 'local', unixepoch()) ON CONFLICT(provider, track_id) DO UPDATE SET sync_state='local', updated_at=excluded.updated_at",
                crate::deps::rusqlite::params![prvstr(id), id.raw_id],
            )
        } else {
            conn.execute("DELETE FROM favorites WHERE provider=?1 AND track_id=?2 AND sync_state='local'", crate::deps::rusqlite::params![prvstr(id), id.raw_id])
        }.map_err(|e| StorageError::Migration(e.to_string()))?;
        Ok(())
    }

    /// 持久化用户添加至 CoverFlow 的卡片列表。
    fn svdecks(&self, decks: &[CoverDeck]) -> Result<(), AppError> {
        let mut conn = self.conn.lock().map_err(|_| StorageError::Corrupt)?;
        let tx = conn.transaction().map_err(|e| StorageError::Migration(e.to_string()))?;
        tx.execute("DELETE FROM coverflow_decks", [])
            .map_err(|e| StorageError::Migration(e.to_string()))?;
        for (pos, deck) in decks.iter().enumerate() {
            let payload = crate::deps::serde_json::to_string(deck)
                .map_err(|e| StorageError::Migration(e.to_string()))?;
            let kind = dckstr(deck.kind);
            tx.execute(
                "INSERT INTO coverflow_decks (pos, deck_id, kind, title, cover_key, source_id, removable, payload_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                crate::deps::rusqlite::params![
                    pos as i64,
                    deck.id,
                    kind,
                    deck.title,
                    deck.cover_key,
                    deck.source_id,
                    if deck.removable { 1i64 } else { 0i64 },
                    payload,
                ],
            )
            .map_err(|e| StorageError::Migration(e.to_string()))?;
        }
        tx.commit().map_err(|e| StorageError::Migration(e.to_string()))?;
        Ok(())
    }

    /// 读取持久化的 CoverFlow 卡片列表。
    fn lddecks(&self) -> Result<Vec<CoverDeck>, AppError> {
        let conn = self.conn.lock().map_err(|_| StorageError::Corrupt)?;
        let mut stmt = conn
            .prepare("SELECT payload_json FROM coverflow_decks ORDER BY pos ASC")
            .map_err(|e| StorageError::Migration(e.to_string()))?;
        let rows = stmt
            .query_map([], |row| row.get::<_, Option<String>>(0))
            .map_err(|e| StorageError::Migration(e.to_string()))?;
        let mut out = Vec::new();
        for row in rows {
            if let Some(json) = row.map_err(|e| StorageError::Migration(e.to_string()))? {
                if let Ok(mut deck) = crate::deps::serde_json::from_str::<CoverDeck>(&json) {
                    for t in &mut deck.tracks {
                        t.title = clnttl(&t.title);
                    }
                    out.push(deck);
                }
            }
        }
        Ok(out)
    }

    /// 保存本地已下载音频文件记录。
    fn svdwnld(&self, rec: &DownloadedFile) -> Result<(), AppError> {
        let conn = self.conn.lock().map_err(|_| StorageError::Corrupt)?;
        conn.execute(
            "INSERT INTO downloaded_tracks (provider, track_id, quality, is_lossless, collection_name, file_path, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, unixepoch()) ON CONFLICT(provider, track_id, quality) DO UPDATE SET is_lossless=excluded.is_lossless, collection_name=excluded.collection_name, file_path=excluded.file_path, updated_at=excluded.updated_at",
            crate::deps::rusqlite::params![
                prvstr(&rec.track_id),
                rec.track_id.raw_id,
                rec.quality.kgstr(),
                if rec.is_lossless { 1i64 } else { 0i64 },
                rec.collection_name,
                rec.file_path,
            ],
        )
        .map_err(|e| StorageError::Migration(e.to_string()))?;
        Ok(())
    }

    /// 查询曲目可用的本地已下载文件记录（无损优先）。
    fn lddwnld(&self, id: &TrackId) -> Result<Option<DownloadedFile>, AppError> {
        let conn = self.conn.lock().map_err(|_| StorageError::Corrupt)?;
        let mut stmt = conn
            .prepare(
                "SELECT quality, is_lossless, collection_name, file_path FROM downloaded_tracks WHERE provider=?1 AND track_id=?2 ORDER BY is_lossless DESC, updated_at DESC",
            )
            .map_err(|e| StorageError::Migration(e.to_string()))?;
        let rows = stmt
            .query_map(crate::deps::rusqlite::params![prvstr(id), id.raw_id], |row| {
                let q: String = row.get(0)?;
                let l: i64 = row.get(1)?;
                let c: String = row.get(2)?;
                let p: String = row.get(3)?;
                Ok((q, l != 0, c, p))
            })
            .map_err(|e| StorageError::Migration(e.to_string()))?;
        for row in rows {
            let (q, is_lossless, collection_name, file_path) =
                row.map_err(|e| StorageError::Migration(e.to_string()))?;
            if Path::new(&file_path).is_file() {
                return Ok(Some(DownloadedFile {
                    track_id: id.clone(),
                    quality: strqlt(&q),
                    is_lossless,
                    collection_name,
                    file_path,
                }));
            }
        }
        Ok(None)
    }
}

/// 转换数据库中的平台标识。
fn prvstr(id: &TrackId) -> &'static str {
    match id.provider { ProviderId::Kugou => "kugou", ProviderId::Local => "local" }
}

/// 转换卡片类型标识。
fn dckstr(kind: DeckKind) -> &'static str {
    match kind {
        DeckKind::Like => "like",
        DeckKind::DailyRecommend => "daily",
        DeckKind::Album => "album",
        DeckKind::Single => "single",
        DeckKind::UserPlaylist => "user_pl",
        DeckKind::RecommendedPlaylist => "rec_pl",
    }
}

/// 将存储音质字符串映射为枚举。
fn strqlt(raw: &str) -> AudioQuality {
    AudioQuality::frmkg(raw).unwrap_or(AudioQuality::High)
}

/// 解析存储中的提供商标识。
fn prvof(raw: &str) -> Option<ProviderId> {
    match raw {
        "kugou" => Some(ProviderId::Kugou),
        "local" => Some(ProviderId::Local),
        _ => None,
    }
}

/// 读取完整曲目行。
fn rdtrk(row: &Row<'_>) -> crate::deps::rusqlite::Result<TrackSummary> {
    let provider: String = row.get(0)?;
    let provider = prvof(&provider).ok_or(crate::deps::rusqlite::Error::InvalidQuery)?;
    let hashes_raw: Option<String> = row.get(9)?;
    let extra_hashes = hashes_raw
        .and_then(|s| crate::deps::serde_json::from_str::<BTreeMap<String, String>>(&s).ok())
        .unwrap_or_default();
    let raw_title: String = row.get(2)?;
    Ok(TrackSummary {
        id: TrackId { provider, raw_id: row.get(1)? },
        title: clnttl(&raw_title),
        artist_text: row.get(3)?,
        cover_key: row.get(4)?,
        duration_ms: row.get::<_, Option<i64>>(5)?.map(|value| value.max(0) as u64),
        album_id: row.get(6)?,
        album_name: row.get(7)?,
        album_audio_id: row.get(8)?,
        extra_hashes,
    })
}

/// 验证持久化设置的有效范围。
fn vldcfg(cfg: &AppSettings) -> Result<(), AppError> {
    if !cfg.audio.volume.is_finite() || !(0.0..=1.0).contains(&cfg.audio.volume) {
        return Err(StorageError::Corrupt.into());
    }
    Ok(())
}

/// 完整写入临时文件后原子替换目标。
pub(crate) fn wrtatom(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|_| StorageError::Permission)?;
    }
    let temporary = path.with_extension(format!("{}.tmp", crate::deps::uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() { let _ = std::fs::remove_file(&temporary); }
    result.map_err(|_| StorageError::Permission.into())
}
