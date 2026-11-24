use std::env;
use std::fmt;
use std::fs;
use std::net::IpAddr;
use std::num::{NonZeroU16, NonZeroU64};
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use serde::Deserialize;

use crate::error::{Result, ThalliumError};

/// Maximum allowed duration in seconds (24 hours) for polling/timeout/drain timers
/// to prevent timer overflow or accidental multi-year stalls.
const MAX_TIMER_SECS: u64 = 86_400;

/// Top-level Thallium configuration loaded from TOML + environment overrides.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub thallium: ThalliumConfig,
    pub minecraft: MinecraftConfig,
    #[serde(default)]
    pub state_machine: StateMachineConfig,
    pub storage: StorageConfig,
    pub database: DatabaseConfig,
}

/// Deployment environment for Thallium.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Environment {
    #[default]
    Development,
    Staging,
    Production,
}

impl fmt::Display for Environment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Development => f.write_str("development"),
            Self::Staging => f.write_str("staging"),
            Self::Production => f.write_str("production"),
        }
    }
}

impl FromStr for Environment {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "development" => Ok(Self::Development),
            "staging" => Ok(Self::Staging),
            "production" => Ok(Self::Production),
            other => Err(format!(
                "invalid environment '{other}'; expected one of: development, staging, production"
            )),
        }
    }
}

/// General application metadata and runtime behavior.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ThalliumConfig {
    /// Display name for this Thallium instance (default: `"Thallium"`).
    pub name: String,
    /// Runtime environment (`development`, `staging`, `production`; default: `development`).
    pub environment: Environment,
    /// Enables verbose debug logging/diagnostics (default: `false`; typically overridden to `true` locally).
    pub debug: bool,
}

impl Default for ThalliumConfig {
    fn default() -> Self {
        Self {
            name: "Thallium".to_string(),
            environment: Environment::Development,
            debug: false,
        }
    }
}

/// Validated Minecraft server host: an IPv4/IPv6 address or DNS / Tailscale MagicDNS hostname.
///
/// Rejects empty strings, placeholders, URL schemes (`http://`), and embedded ports (`host:25565`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub struct ServerHost(String);

impl ServerHost {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_ip(&self) -> bool {
        self.0.parse::<IpAddr>().is_ok()
    }
}

impl fmt::Display for ServerHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for ServerHost {
    type Error = String;

    fn try_from(value: String) -> std::result::Result<Self, Self::Error> {
        if value.is_empty() || value.trim() != value {
            return Err(
                "host must be non-empty without leading or trailing whitespace".to_string(),
            );
        }
        if value.contains("<insert") {
            return Err("host still contains an unreplaced placeholder".to_string());
        }
        if value.contains("://")
            || value.contains('/')
            || value.chars().any(|c| c.is_whitespace() || c.is_control())
        {
            return Err(format!(
                "host '{value}' must be a bare hostname or IP address without scheme or whitespace"
            ));
        }

        // Accept valid IPv4 or IPv6 addresses directly.
        if value.parse::<IpAddr>().is_ok() {
            return Ok(Self(value));
        }

        // Otherwise validate as a DNS / Tailscale MagicDNS hostname (e.g. "smp-node" or "smp-node.tailnet.ts.net").
        if value.contains(':') {
            return Err(format!(
                "host '{value}' appears to include a port; set `minecraft.port` separately"
            ));
        }
        if value.len() > 253 {
            return Err("hostname exceeds maximum length of 253 characters".to_string());
        }
        for label in value.split('.') {
            if label.is_empty() || label.len() > 63 {
                return Err(format!("invalid hostname label in '{value}'"));
            }
            if label.starts_with('-') || label.ends_with('-') {
                return Err(format!(
                    "hostname label '{label}' cannot start or end with '-'"
                ));
            }
            if !label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            {
                return Err(format!(
                    "hostname label '{label}' contains invalid characters"
                ));
            }
        }

        Ok(Self(value))
    }
}

/// Validated non-empty filesystem path that cannot contain unreplaced template placeholders.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "PathBuf")]
pub struct ValidatedPath(PathBuf);

impl ValidatedPath {
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

impl Deref for ValidatedPath {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<Path> for ValidatedPath {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl TryFrom<PathBuf> for ValidatedPath {
    type Error = String;

    fn try_from(value: PathBuf) -> std::result::Result<Self, Self::Error> {
        let lossy = value.to_string_lossy();
        if lossy.trim().is_empty() {
            return Err("path must not be empty".to_string());
        }
        if lossy.contains("<insert") {
            return Err("path still contains an unreplaced placeholder".to_string());
        }
        Ok(Self(value))
    }
}

/// Validated S3/R2-compatible bucket name (3..=63 chars, `[a-z0-9-]`, alphanumeric ends).
///
/// Enforcing strict bucket syntax at deserialization time prevents filesystem path traversal
/// (e.g. `../`, `/`, `\`) when `LocalBucketStore` joins `storage_root` with `bucket`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub struct BucketName(String);

impl BucketName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BucketName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for BucketName {
    type Error = String;

    fn try_from(value: String) -> std::result::Result<Self, Self::Error> {
        if !(3..=63).contains(&value.len()) {
            return Err(format!(
                "bucket name '{value}' must be between 3 and 63 characters"
            ));
        }
        let bytes = value.as_bytes();
        let valid_end = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit();
        if !valid_end(bytes[0]) || !valid_end(bytes[bytes.len() - 1]) {
            return Err(format!(
                "bucket name '{value}' must start and end with a lowercase letter or digit"
            ));
        }
        if !bytes
            .iter()
            .all(|&b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err(format!(
                "bucket name '{value}' may only contain lowercase letters, digits, and hyphens"
            ));
        }
        Ok(Self(value))
    }
}

/// Validated Postgres connection URL whose credentials are redacted in `Debug` output.
///
/// Intentionally does not implement `Display` so secrets are never accidentally formatted into logs.
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub struct DatabaseUrl(String);

impl DatabaseUrl {
    /// Returns the raw connection string for passing to `sqlx`.
    pub fn expose_secret(&self) -> &str {
        &self.0
    }

    /// Returns a credential-redacted representation safe for logging.
    pub fn redacted(&self) -> String {
        let Some(scheme_end) = self.0.find("://") else {
            return "<redacted>".to_string();
        };
        let scheme = &self.0[..scheme_end + 3];
        let rest = &self.0[scheme_end + 3..];
        let authority_end = rest.find('/').unwrap_or(rest.len());
        let authority = &rest[..authority_end];
        let suffix = &rest[authority_end..];

        if let Some(at_pos) = authority.rfind('@') {
            format!("{scheme}***@{}{suffix}", &authority[at_pos + 1..])
        } else {
            self.0.clone()
        }
    }
}

impl fmt::Debug for DatabaseUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("DatabaseUrl")
            .field(&self.redacted())
            .finish()
    }
}

impl TryFrom<String> for DatabaseUrl {
    type Error = String;

    fn try_from(value: String) -> std::result::Result<Self, Self::Error> {
        if value.is_empty() || value.trim() != value {
            return Err(
                "database_url must be non-empty without leading or trailing whitespace".to_string(),
            );
        }
        if value.contains("<insert") {
            return Err("database_url still contains an unreplaced placeholder".to_string());
        }
        if value.chars().any(|c| c.is_control()) {
            return Err("database_url must not contain control characters".to_string());
        }

        let rest = value
            .strip_prefix("postgres://")
            .or_else(|| value.strip_prefix("postgresql://"))
            .ok_or_else(|| {
                "database_url must start with 'postgres://' or 'postgresql://'".to_string()
            })?;

        let authority = rest.split('/').next().unwrap_or("");
        let host_port = authority.rsplit('@').next().unwrap_or("");
        if host_port.is_empty() {
            return Err("database_url must include a non-empty host".to_string());
        }

        Ok(Self(value))
    }
}

/// Minecraft server SLP connection and playerdata filesystem paths.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MinecraftConfig {
    /// Target hostname or IP for off-server Server List Ping (SLP).
    /// Supports IPv4/IPv6, FQDNs, and short Tailscale MagicDNS hostnames (default: `"127.0.0.1"`).
    #[serde(default = "default_host")]
    pub host: ServerHost,
    /// Target TCP port for SLP (non-zero; default: `25565`).
    #[serde(default = "default_port")]
    pub port: NonZeroU16,
    /// Timeout in seconds for each SLP TCP handshake/status exchange (must be < `poll_interval_secs`; default: `5`).
    #[serde(default = "default_ping_timeout_secs")]
    pub ping_timeout_secs: NonZeroU64,
    /// Required directory containing `<uuid>.dat` player NBT files
    /// (e.g., `world/players/data` on Minecraft 26.3+ or `world/playerdata` on older versions).
    pub playerdata_dir: ValidatedPath,
    /// Optional path to `usercache.json` for resolving player UUIDs to usernames.
    #[serde(default)]
    pub usercache_path: Option<ValidatedPath>,
}

impl MinecraftConfig {
    pub fn ping_timeout(&self) -> Duration {
        Duration::from_secs(self.ping_timeout_secs.get())
    }

    /// Formats `host:port` (bracketing IPv6 literals) for `TcpStream::connect`.
    pub fn socket_addr(&self) -> String {
        if matches!(self.host.as_str().parse::<IpAddr>(), Ok(IpAddr::V6(_))) {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }
}

/// Polling and drain timing for the off-server player-count state machine.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StateMachineConfig {
    /// Interval in seconds between SLP status polls (must be > `ping_timeout_secs`; default: `15`).
    pub poll_interval_secs: NonZeroU64,
    /// Delay in seconds after detecting 0 players before reading `.dat` files so the server finishes flushing (default: `2`).
    pub drain_delay_secs: u64,
}

impl Default for StateMachineConfig {
    fn default() -> Self {
        Self {
            poll_interval_secs: NonZeroU64::new(15).expect("15 is non-zero"),
            drain_delay_secs: 2,
        }
    }
}

impl StateMachineConfig {
    pub fn poll_interval(&self) -> Duration {
        Duration::from_secs(self.poll_interval_secs.get())
    }

    pub fn drain_delay(&self) -> Duration {
        Duration::from_secs(self.drain_delay_secs)
    }
}

/// Local S3/R2-style object storage settings.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageConfig {
    /// Required root directory under which bucket folders are created (relative or absolute).
    pub storage_root: ValidatedPath,
    /// S3/R2-compatible bucket name for playerdata snapshots (default: `"playerdata"`).
    #[serde(default = "default_bucket")]
    pub bucket: BucketName,
}

impl StorageConfig {
    /// Returns the resolved local directory path for the configured bucket.
    pub fn bucket_path(&self) -> PathBuf {
        self.storage_root.join(self.bucket.as_str())
    }
}

/// Postgres secondary index connection settings.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatabaseConfig {
    /// Required Postgres connection URI (overridable via the `DATABASE_URL` environment variable).
    pub database_url: DatabaseUrl,
}

fn default_host() -> ServerHost {
    ServerHost("127.0.0.1".to_string())
}

fn default_port() -> NonZeroU16 {
    NonZeroU16::new(25565).expect("25565 is non-zero")
}

fn default_ping_timeout_secs() -> NonZeroU64 {
    NonZeroU64::new(5).expect("5 is non-zero")
}

fn default_bucket() -> BucketName {
    BucketName("playerdata".to_string())
}

impl Config {
    /// Load configuration using `THALLIUM_CONFIG`, then `config.local.toml`,
    /// falling back to `config.toml`. Applies environment overrides afterward.
    pub fn load() -> Result<Self> {
        let path = Self::resolve_config_path_with(|k| env::var(k).ok());
        Self::from_path(path)
    }

    /// Load configuration from a specific TOML file and apply process env overrides.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let raw = fs::read_to_string(path).map_err(|source| ThalliumError::FileSystem {
            path: path.to_path_buf(),
            source,
        })?;

        Self::from_toml_str_with_env(&raw, |k| env::var(k).ok())
    }

    /// Parse and validate configuration from a TOML string without reading environment variables.
    pub fn from_toml_str(raw: &str) -> Result<Self> {
        Self::from_toml_str_with_env(raw, |_| None)
    }

    /// Parse configuration from TOML, apply overrides from `env_get`, and run cross-field validation.
    pub fn from_toml_str_with_env<F>(raw: &str, env_get: F) -> Result<Self>
    where
        F: Fn(&str) -> Option<String>,
    {
        let mut config: Self = toml::from_str(raw)?;
        config.apply_env_overrides(env_get)?;
        config.validate()?;
        Ok(config)
    }

    /// Verifies that configured filesystem paths exist and have the expected kind at runtime.
    pub fn verify_paths(&self) -> Result<()> {
        if !self.minecraft.playerdata_dir.is_dir() {
            return Err(ThalliumError::ConfigValidation {
                field: "minecraft.playerdata_dir".to_string(),
                reason: format!(
                    "directory '{}' does not exist or is not a directory",
                    self.minecraft.playerdata_dir.display()
                ),
            });
        }

        if let Some(usercache) = &self.minecraft.usercache_path
            && usercache.exists()
            && !usercache.is_file()
        {
            return Err(ThalliumError::ConfigValidation {
                field: "minecraft.usercache_path".to_string(),
                reason: format!("'{}' exists but is not a regular file", usercache.display()),
            });
        }

        Ok(())
    }

    fn resolve_config_path_with<F>(env_get: F) -> PathBuf
    where
        F: Fn(&str) -> Option<String>,
    {
        if let Some(explicit) = env_get("THALLIUM_CONFIG") {
            let trimmed = explicit.trim();
            if !trimmed.is_empty() {
                return PathBuf::from(trimmed);
            }
        }

        let local = PathBuf::from("config.local.toml");
        if local.exists() {
            return local;
        }

        PathBuf::from("config.toml")
    }

    fn apply_env_overrides<F>(&mut self, env_get: F) -> Result<()>
    where
        F: Fn(&str) -> Option<String>,
    {
        if let Some(raw_url) = env_get("DATABASE_URL")
            && !raw_url.trim().is_empty()
        {
            self.database.database_url = DatabaseUrl::try_from(raw_url).map_err(|reason| {
                ThalliumError::ConfigValidation {
                    field: "DATABASE_URL".to_string(),
                    reason,
                }
            })?;
        }

        if let Some(raw_env) = env_get("THALLIUM_ENV")
            && !raw_env.trim().is_empty()
        {
            self.thallium.environment = raw_env.parse::<Environment>().map_err(|reason| {
                ThalliumError::ConfigValidation {
                    field: "THALLIUM_ENV".to_string(),
                    reason,
                }
            })?;
        }

        if let Some(raw_debug) = env_get("THALLIUM_DEBUG")
            && !raw_debug.trim().is_empty()
        {
            self.thallium.debug =
                parse_env_bool(&raw_debug).ok_or_else(|| ThalliumError::ConfigValidation {
                    field: "THALLIUM_DEBUG".to_string(),
                    reason: format!("invalid boolean '{raw_debug}'; expected true/false or 1/0"),
                })?;
        }

        Ok(())
    }

    fn validate(&self) -> Result<()> {
        if self.thallium.name.trim().is_empty() || self.thallium.name.contains("<insert") {
            return Err(ThalliumError::ConfigValidation {
                field: "thallium.name".to_string(),
                reason: "must be non-empty and not contain a placeholder".to_string(),
            });
        }

        if self.minecraft.ping_timeout_secs.get() > MAX_TIMER_SECS {
            return Err(ThalliumError::ConfigValidation {
                field: "minecraft.ping_timeout_secs".to_string(),
                reason: format!("must not exceed {MAX_TIMER_SECS} seconds"),
            });
        }

        if self.state_machine.poll_interval_secs.get() > MAX_TIMER_SECS {
            return Err(ThalliumError::ConfigValidation {
                field: "state_machine.poll_interval_secs".to_string(),
                reason: format!("must not exceed {MAX_TIMER_SECS} seconds"),
            });
        }

        if self.state_machine.drain_delay_secs > MAX_TIMER_SECS {
            return Err(ThalliumError::ConfigValidation {
                field: "state_machine.drain_delay_secs".to_string(),
                reason: format!("must not exceed {MAX_TIMER_SECS} seconds"),
            });
        }

        if self.minecraft.ping_timeout_secs >= self.state_machine.poll_interval_secs {
            return Err(ThalliumError::ConfigValidation {
                field: "minecraft.ping_timeout_secs".to_string(),
                reason: format!(
                    "ping_timeout_secs ({}) must be strictly less than state_machine.poll_interval_secs ({}) to avoid overlapping polls",
                    self.minecraft.ping_timeout_secs, self.state_machine.poll_interval_secs
                ),
            });
        }

        if self.storage.storage_root == self.minecraft.playerdata_dir {
            return Err(ThalliumError::ConfigValidation {
                field: "storage.storage_root".to_string(),
                reason: "must not point to the same directory as minecraft.playerdata_dir"
                    .to_string(),
            });
        }

        Ok(())
    }
}

fn parse_env_bool(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" => Some(true),
        "false" | "0" | "no" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_CONFIG_TOML: &str = r#"
[thallium]
name = "Thallium"
environment = "development"
debug = true

[minecraft]
host = "smp-node"
port = 25565
ping_timeout_secs = 5
playerdata_dir = "/srv/minecraft/world/players/data"
usercache_path = "/srv/minecraft/usercache.json"

[state_machine]
poll_interval_secs = 15
drain_delay_secs = 2

[storage]
storage_root = "./buckets"
bucket = "smp-playerdata"

[database]
database_url = "postgres://thallium:changeme@localhost:5432/thallium"
"#;

    #[test]
    fn parses_full_configuration() {
        let cfg = Config::from_toml_str(SAMPLE_CONFIG_TOML).expect("valid sample config");
        assert_eq!(cfg.thallium.name, "Thallium");
        assert_eq!(cfg.thallium.environment, Environment::Development);
        assert!(cfg.thallium.debug);
        assert_eq!(cfg.minecraft.host.as_str(), "smp-node");
        assert_eq!(cfg.minecraft.port.get(), 25565);
        assert_eq!(cfg.minecraft.socket_addr(), "smp-node:25565");
        assert_eq!(cfg.minecraft.ping_timeout(), Duration::from_secs(5));
        assert_eq!(
            cfg.minecraft.playerdata_dir.as_path(),
            Path::new("/srv/minecraft/world/players/data")
        );
        assert_eq!(
            cfg.minecraft.usercache_path.as_deref(),
            Some(Path::new("/srv/minecraft/usercache.json"))
        );
        assert_eq!(cfg.state_machine.poll_interval(), Duration::from_secs(15));
        assert_eq!(cfg.state_machine.drain_delay(), Duration::from_secs(2));
        assert_eq!(cfg.storage.bucket.as_str(), "smp-playerdata");
        assert_eq!(
            cfg.storage.bucket_path(),
            PathBuf::from("./buckets/smp-playerdata")
        );
        assert_eq!(
            cfg.database.database_url.expose_secret(),
            "postgres://thallium:changeme@localhost:5432/thallium"
        );
    }

    #[test]
    fn rejects_config_example_placeholders() {
        let example = include_str!("../config.example.toml");
        let err = Config::from_toml_str(example).expect_err("example placeholders must fail");
        assert!(matches!(err, ThalliumError::TomlParse(_)));
    }

    #[test]
    fn applies_defaults_for_omitted_optional_fields() {
        let minimal = r#"
[minecraft]
playerdata_dir = "/srv/minecraft/world/players/data"

[storage]
storage_root = "/var/lib/thallium/buckets"

[database]
database_url = "postgresql://localhost:5432/thallium"
"#;
        let cfg = Config::from_toml_str(minimal).expect("minimal config should use defaults");
        assert_eq!(cfg.thallium.name, "Thallium");
        assert_eq!(cfg.thallium.environment, Environment::Development);
        assert!(!cfg.thallium.debug);
        assert_eq!(cfg.minecraft.host.as_str(), "127.0.0.1");
        assert_eq!(cfg.minecraft.port.get(), 25565);
        assert_eq!(cfg.minecraft.ping_timeout(), Duration::from_secs(5));
        assert!(cfg.minecraft.usercache_path.is_none());
        assert_eq!(cfg.state_machine.poll_interval(), Duration::from_secs(15));
        assert_eq!(cfg.state_machine.drain_delay(), Duration::from_secs(2));
        assert_eq!(cfg.storage.bucket.as_str(), "playerdata");
    }

    #[test]
    fn redacts_database_credentials_in_debug_output() {
        let cfg = Config::from_toml_str(SAMPLE_CONFIG_TOML).unwrap();
        let debug_str = format!("{cfg:?}");
        assert!(
            !debug_str.contains("changeme"),
            "Debug output leaked database password: {debug_str}"
        );
        assert!(debug_str.contains("postgres://***@localhost:5432/thallium"));
    }

    #[test]
    fn rejects_zero_port_and_zero_timers() {
        let zero_port = SAMPLE_CONFIG_TOML.replace("port = 25565", "port = 0");
        assert!(Config::from_toml_str(&zero_port).is_err());

        let zero_poll =
            SAMPLE_CONFIG_TOML.replace("poll_interval_secs = 15", "poll_interval_secs = 0");
        assert!(Config::from_toml_str(&zero_poll).is_err());

        let zero_timeout =
            SAMPLE_CONFIG_TOML.replace("ping_timeout_secs = 5", "ping_timeout_secs = 0");
        assert!(Config::from_toml_str(&zero_timeout).is_err());
    }

    #[test]
    fn rejects_ping_timeout_gte_poll_interval() {
        let overlapping =
            SAMPLE_CONFIG_TOML.replace("ping_timeout_secs = 5", "ping_timeout_secs = 15");
        let err = Config::from_toml_str(&overlapping).expect_err("overlapping poll must fail");
        assert!(matches!(
            err,
            ThalliumError::ConfigValidation { ref field, .. } if field == "minecraft.ping_timeout_secs"
        ));
    }

    #[test]
    fn rejects_bucket_path_traversal_and_invalid_names() {
        for bad in ["../escape", "a/b", "UPPER", "ab", "-leading", "trailing-"] {
            let raw = SAMPLE_CONFIG_TOML.replace("smp-playerdata", bad);
            assert!(
                Config::from_toml_str(&raw).is_err(),
                "bucket '{bad}' should have been rejected"
            );
        }
    }

    #[test]
    fn validates_hosts_and_brackets_ipv6() {
        for bad in [
            "smp-node:25565",
            "http://smp-node",
            "",
            "bad host",
            "-smp-node",
            "smp-node..ts",
        ] {
            let raw =
                SAMPLE_CONFIG_TOML.replace(r#"host = "smp-node""#, &format!(r#"host = "{bad}""#));
            assert!(
                Config::from_toml_str(&raw).is_err(),
                "host '{bad}' should have been rejected"
            );
        }

        let ipv6 = SAMPLE_CONFIG_TOML.replace(r#"host = "smp-node""#, r#"host = "::1""#);
        let cfg = Config::from_toml_str(&ipv6).expect("IPv6 host should be accepted");
        assert_eq!(cfg.minecraft.socket_addr(), "[::1]:25565");

        let tailnet = SAMPLE_CONFIG_TOML.replace(
            r#"host = "smp-node""#,
            r#"host = "smp-node.tail12345.ts.net""#,
        );
        let cfg = Config::from_toml_str(&tailnet).expect("Tailscale FQDN should be accepted");
        assert_eq!(
            cfg.minecraft.socket_addr(),
            "smp-node.tail12345.ts.net:25565"
        );
    }

    #[test]
    fn rejects_unknown_toml_fields() {
        let typo = SAMPLE_CONFIG_TOML.replace("poll_interval_secs = 15", "poll_interval_sec = 15");
        assert!(Config::from_toml_str(&typo).is_err());
    }

    #[test]
    fn rejects_placeholder_in_optional_usercache_path() {
        let raw = SAMPLE_CONFIG_TOML.replace(
            "/srv/minecraft/usercache.json",
            "<insert path to usercache.json here!>",
        );
        assert!(Config::from_toml_str(&raw).is_err());
    }

    #[test]
    fn rejects_non_postgres_database_url() {
        let raw = SAMPLE_CONFIG_TOML.replace(
            "postgres://thallium:changeme@localhost:5432/thallium",
            "mysql://root:pw@localhost:3306/thallium",
        );
        assert!(Config::from_toml_str(&raw).is_err());
    }

    #[test]
    fn applies_and_validates_env_overrides() {
        let cfg = Config::from_toml_str_with_env(SAMPLE_CONFIG_TOML, |key| match key {
            "DATABASE_URL" => {
                Some("postgresql://prod:secret@db.internal:5432/thallium".to_string())
            }
            "THALLIUM_ENV" => Some("production".to_string()),
            "THALLIUM_DEBUG" => Some("false".to_string()),
            _ => None,
        })
        .expect("valid env overrides");

        assert_eq!(cfg.thallium.environment, Environment::Production);
        assert!(!cfg.thallium.debug);
        assert_eq!(
            cfg.database.database_url.expose_secret(),
            "postgresql://prod:secret@db.internal:5432/thallium"
        );

        let bad_env = Config::from_toml_str_with_env(SAMPLE_CONFIG_TOML, |key| match key {
            "THALLIUM_ENV" => Some("prodcution".to_string()),
            _ => None,
        });
        assert!(matches!(
            bad_env,
            Err(ThalliumError::ConfigValidation { field, .. }) if field == "THALLIUM_ENV"
        ));

        let bad_debug = Config::from_toml_str_with_env(SAMPLE_CONFIG_TOML, |key| match key {
            "THALLIUM_DEBUG" => Some("maybe".to_string()),
            _ => None,
        });
        assert!(matches!(
            bad_debug,
            Err(ThalliumError::ConfigValidation { field, .. }) if field == "THALLIUM_DEBUG"
        ));
    }

    #[test]
    fn rejects_storage_root_matching_playerdata_dir() {
        let raw = SAMPLE_CONFIG_TOML.replace(
            r#"storage_root = "./buckets""#,
            r#"storage_root = "/srv/minecraft/world/players/data""#,
        );
        let err = Config::from_toml_str(&raw).expect_err("overlapping dirs must fail");
        assert!(matches!(
            err,
            ThalliumError::ConfigValidation { ref field, .. } if field == "storage.storage_root"
        ));
    }
}
