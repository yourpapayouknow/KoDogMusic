use crate::deps::rusqlite::Connection;
use crate::domain::error::{AppError, StorageError};

/// 执行数据库表结构迁移与初始化
pub fn mgtr(conn: &Connection) -> Result<(), AppError> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS tracks (
            provider     TEXT NOT NULL,
            track_id     TEXT NOT NULL,
            title        TEXT NOT NULL,
            artist_text  TEXT NOT NULL,
            album_text   TEXT,
            duration_ms  INTEGER,
            cover_key    TEXT,
            updated_at   INTEGER NOT NULL,
            PRIMARY KEY (provider, track_id)
        );

        CREATE TABLE IF NOT EXISTS queue_items (
            pos          INTEGER PRIMARY KEY,
            provider     TEXT NOT NULL,
            track_id     TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS playback_state (
            id           INTEGER PRIMARY KEY CHECK (id = 1),
            cursor_pos   INTEGER,
            position_ms  INTEGER NOT NULL DEFAULT 0,
            volume       REAL NOT NULL DEFAULT 0.5,
            repeat_mode  TEXT NOT NULL DEFAULT 'all',
            shuffle      INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS favorites (
            provider     TEXT NOT NULL,
            track_id     TEXT NOT NULL,
            sync_state   TEXT NOT NULL,
            updated_at   INTEGER NOT NULL,
            PRIMARY KEY (provider, track_id)
        );

        CREATE TABLE IF NOT EXISTS cache_entries (
            cache_key    TEXT PRIMARY KEY,
            kind         TEXT NOT NULL,
            rel_path     TEXT NOT NULL,
            size_bytes   INTEGER NOT NULL,
            last_used_at INTEGER NOT NULL,
            expires_at   INTEGER
        );

        CREATE TABLE IF NOT EXISTS downloads (
            job_id       TEXT PRIMARY KEY,
            provider     TEXT NOT NULL,
            track_id     TEXT NOT NULL,
            quality      TEXT NOT NULL,
            dest_path    TEXT NOT NULL,
            state        TEXT NOT NULL,
            bytes_done   INTEGER NOT NULL DEFAULT 0,
            bytes_total  INTEGER
        );

        CREATE TABLE IF NOT EXISTS coverflow_decks (
            pos          INTEGER PRIMARY KEY,
            deck_id      TEXT NOT NULL,
            kind         TEXT NOT NULL,
            title        TEXT NOT NULL,
            cover_key    TEXT,
            source_id    TEXT NOT NULL,
            removable    INTEGER NOT NULL DEFAULT 1,
            payload_json TEXT
        );

        CREATE TABLE IF NOT EXISTS downloaded_tracks (
            provider        TEXT NOT NULL,
            track_id        TEXT NOT NULL,
            quality         TEXT NOT NULL,
            is_lossless     INTEGER NOT NULL,
            collection_name TEXT NOT NULL,
            file_path       TEXT NOT NULL,
            updated_at      INTEGER NOT NULL,
            PRIMARY KEY (provider, track_id, quality)
        );

        CREATE TABLE IF NOT EXISTS schema_migrations (
            version      INTEGER PRIMARY KEY,
            applied_at   INTEGER NOT NULL
        );
        ",
    )
    .map_err(|e| StorageError::Migration(e.to_string()))?;

    addcol(conn, "tracks", "album_id", "TEXT")?;
    addcol(conn, "tracks", "album_name", "TEXT")?;
    addcol(conn, "tracks", "album_audio_id", "INTEGER")?;
    addcol(conn, "tracks", "extra_hashes", "TEXT")?;

    Ok(())
}

/// 按列存在性幂等追加表列。
fn addcol(conn: &Connection, table: &str, col: &str, typ: &str) -> Result<(), AppError> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|e| StorageError::Migration(e.to_string()))?;
    let cols = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| StorageError::Migration(e.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| StorageError::Migration(e.to_string()))?;
    if !cols.iter().any(|existing| existing == col) {
        conn.execute(&format!("ALTER TABLE {table} ADD COLUMN {col} {typ}"), [])
            .map_err(|e| StorageError::Migration(e.to_string()))?;
    }
    Ok(())
}
