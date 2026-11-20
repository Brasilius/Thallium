use std::path::PathBuf;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, ThalliumError>;

#[derive(Debug, Error)]
pub enum ThalliumError {
    // 1. Hardware, OS & I/O Failures
    #[error("I/O failure: {0}")]
    Io(#[from] std::io::Error),

    #[error("File system error on path '{path}': {source}")]
    FileSystem {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "Memory allocation / decompression bound exceeded while processing '{context}': {requested_bytes} bytes requested (limit: {max_bytes} bytes)"
    )]
    MemoryExhausted {
        context: String,
        requested_bytes: usize,
        max_bytes: usize,
    },

    #[error(
        "Storage quota or disk capacity reached in bucket '{bucket}' (available: {available_bytes} bytes, required: {required_bytes} bytes)"
    )]
    MaxStorageReached {
        bucket: String,
        available_bytes: u64,
        required_bytes: u64,
    },

    #[error(
        "Partial or torn write detected for '{path}': expected {expected_bytes} bytes, found {actual_bytes} bytes"
    )]
    CorruptedWrite {
        path: PathBuf,
        expected_bytes: u64,
        actual_bytes: u64,
    },

    // 2. SMP State Machine & Concurrency / Race Conditions
    #[error(
        "State machine race condition: invalid transition from '{from}' to '{to}' (reason: {reason})"
    )]
    StateTransitionRace {
        from: String,
        to: String,
        reason: String,
    },

    #[error(
        "Snapshot aborted due to player churn (combat log / rapid reconnect detected: {active_players} player(s) rejoined during drain window)"
    )]
    PlayerChurnAbort { active_players: u32 },

    #[error(
        "Concurrent file modification detected on '{path}': Minecraft server was still flushing playerdata (.dat_old lock or mid-read mtime drift)"
    )]
    ConcurrentPlayerDataWrite { path: PathBuf },

    #[error("Snapshot pipeline already in flight for drain event '{event_id}'")]
    PipelineAlreadyInFlight { event_id: String },

    #[error("Server List Ping (SLP) failure against {host}:{port}: {reason}")]
    ServerPing {
        host: String,
        port: u16,
        reason: String,
    },

    // 3. Parsing, Deserialization & Structural SMP Limitations
    #[error("Configuration TOML parse failure: {0}")]
    TomlParse(#[from] toml::de::Error),

    #[error("Configuration validation failed for '{field}': {reason}")]
    ConfigValidation { field: String, reason: String },

    #[error("JSON serialization/deserialization failure: {0}")]
    Json(#[from] serde_json::Error),

    #[error("NBT binary parse failure: {0}")]
    NbtParse(#[from] fastnbt::error::Error),

    #[error("Malformed playerdata file for '{player_uuid}' at '{path}': {reason}")]
    MalformedPlayerData {
        player_uuid: String,
        path: PathBuf,
        reason: String,
    },

    #[error(
        "NBT nesting or payload limit exceeded for player '{player_uuid}' in slot {slot} (depth: {depth}, max_depth: {max_depth}) — possible nested container / book-ban payload"
    )]
    NbtPayloadOverflow {
        player_uuid: String,
        slot: i32,
        depth: usize,
        max_depth: usize,
    },

    #[error("Unrecognized or invalid inventory slot index {slot} for player '{player_uuid}'")]
    InvalidInventorySlot { player_uuid: String, slot: i32 },

    #[error("Unsupported or missing Minecraft DataVersion ({data_version:?}) in '{path}'")]
    UnsupportedDataVersion {
        path: PathBuf,
        data_version: Option<i32>,
    },

    // 4. Object Storage & Postgres Secondary Index Failures
    #[error("Database operation failed: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Database migration failed: {0}")]
    DatabaseMigration(#[from] sqlx::migrate::MigrateError),

    #[error(
        "Object storage integrity failure for '{bucket}/{key}': expected ETag '{expected_etag}', computed '{actual_etag}'"
    )]
    ObjectIntegrityMismatch {
        bucket: String,
        key: String,
        expected_etag: String,
        actual_etag: String,
    },

    #[error(
        "Split-brain storage state: object '{bucket}/{key}' committed to disk, but Postgres index transaction failed: {reason}"
    )]
    StorageIndexDesync {
        bucket: String,
        key: String,
        reason: String,
    },
}
