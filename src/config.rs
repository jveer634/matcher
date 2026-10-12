use std::collections::HashSet;
use std::fmt;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::orderbook::UDecimal;
use crate::wal::pruner::WalPrunerConfig;
use crate::wal::SyncPolicy;

/// Errors encountered while loading or validating the application configuration.
#[derive(Debug)]
pub enum ConfigError {
    /// Failure while loading or deserializing settings through the `config` crate.
    Load(config::ConfigError),
    /// Domain-specific invariant validation failure.
    Validation(String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Load(e) => write!(f, "Configuration load error: {e}"),
            ConfigError::Validation(e) => write!(f, "Configuration validation error: {e}"),
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ConfigError::Load(e) => Some(e),
            ConfigError::Validation(_) => None,
        }
    }
}

impl From<config::ConfigError> for ConfigError {
    fn from(err: config::ConfigError) -> Self {
        ConfigError::Load(err)
    }
}

/// Server network configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Host or IP address to bind to (e.g. `0.0.0.0` or `127.0.0.1`).
    #[serde(default = "default_host")]
    pub host: String,
    /// TCP port to listen on.
    #[serde(default = "default_port")]
    pub port: u16,
}

fn default_host() -> String {
    "0.0.0.0".to_string()
}

const fn default_port() -> u16 {
    50051
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
        }
    }
}

impl ServerConfig {
    /// Validates the server network configuration.
    pub fn validate(&self) -> Result<(), String> {
        if self.host.trim().is_empty() {
            return Err("Server host cannot be empty".to_string());
        }
        if self.port == 0 {
            return Err("Server port must be non-zero (1..=65535)".to_string());
        }
        self.socket_addr()
            .map_err(|e| format!("Invalid server address '{}:{}': {e}", self.host, self.port))?;
        Ok(())
    }

    /// Resolves the socket address for the server.
    pub fn socket_addr(&self) -> Result<SocketAddr, std::net::AddrParseError> {
        format!("{}:{}", self.host, self.port).parse()
    }
}

/// Write-Ahead Log (WAL) persistence configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalConfig {
    /// Base directory where WAL segments and snapshots are stored.
    #[serde(default = "default_wal_dir")]
    pub dir: PathBuf,
    /// Durability policy: `always` (fsync per append) or `none` (buffered page cache).
    #[serde(default)]
    pub sync_policy: SyncPolicy,
}

fn default_wal_dir() -> PathBuf {
    PathBuf::from("data/wal")
}

impl Default for WalConfig {
    fn default() -> Self {
        Self {
            dir: default_wal_dir(),
            sync_policy: SyncPolicy::None,
        }
    }
}

impl WalConfig {
    /// Validates the WAL configuration.
    pub fn validate(&self) -> Result<(), String> {
        if self.dir.as_os_str().is_empty() {
            return Err("WAL directory path cannot be empty".to_string());
        }
        Ok(())
    }
}

/// Background WAL checkpointer and segment pruner configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrunerConfig {
    /// Periodic cleanup interval in seconds.
    #[serde(default = "default_pruner_interval_secs")]
    pub interval_secs: u64,
    /// Number of latest snapshot files to retain (default: 2).
    #[serde(default = "default_retain_snapshots")]
    pub retain_snapshots: usize,
}

const fn default_pruner_interval_secs() -> u64 {
    600 // 10 minutes
}

const fn default_retain_snapshots() -> usize {
    2
}

impl Default for PrunerConfig {
    fn default() -> Self {
        Self {
            interval_secs: default_pruner_interval_secs(),
            retain_snapshots: default_retain_snapshots(),
        }
    }
}

impl PrunerConfig {
    /// Validates the pruner configuration.
    pub fn validate(&self) -> Result<(), String> {
        if self.interval_secs == 0 {
            return Err("WAL pruner interval must be at least 1 second (> 0)".to_string());
        }
        if self.retain_snapshots == 0 {
            return Err("WAL pruner retain_snapshots must be at least 1 for safe fallback durability".to_string());
        }
        Ok(())
    }

    /// Returns the pruning interval as a `std::time::Duration`.
    #[must_use]
    pub fn interval(&self) -> Duration {
        Duration::from_secs(self.interval_secs)
    }

    /// Converts this pruner config into a `WalPrunerConfig`.
    #[must_use]
    pub fn to_wal_pruner_config(&self, base_dir: impl AsRef<Path>) -> WalPrunerConfig {
        WalPrunerConfig {
            base_dir: base_dir.as_ref().to_path_buf(),
            interval: self.interval(),
            retain_snapshots: self.retain_snapshots,
        }
    }
}

/// Initial symbol shard definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolConfig {
    /// Trading symbol pair (e.g. `BTC-USDT`).
    pub symbol: String,
    /// Optional initial reference market price.
    #[serde(default)]
    pub initial_price: Option<Decimal>,
}

impl SymbolConfig {
    /// Creates a new symbol configuration.
    pub fn new(symbol: impl Into<String>, initial_price: Option<Decimal>) -> Self {
        Self {
            symbol: symbol.into(),
            initial_price,
        }
    }

    /// Validates the symbol configuration.
    pub fn validate(&self) -> Result<(), String> {
        let trimmed = self.symbol.trim();
        if trimmed.is_empty() {
            return Err("Symbol identifier cannot be empty".to_string());
        }
        if let Some(price) = self.initial_price {
            if price <= Decimal::ZERO {
                return Err(format!(
                    "Initial price for symbol '{}' must be strictly positive (> 0), got {price}",
                    self.symbol
                ));
            }
        }
        Ok(())
    }

    /// Converts initial price to `UDecimal`.
    pub fn initial_udecimal_price(&self) -> Result<Option<UDecimal>, String> {
        match self.initial_price {
            Some(price) => UDecimal::new_strictly_positive(price).map(Some),
            None => Ok(None),
        }
    }
}

/// Root application configuration for Matcher.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    /// Server network configuration.
    #[serde(default)]
    pub server: ServerConfig,
    /// Write-Ahead Log persistence configuration.
    #[serde(default)]
    pub wal: WalConfig,
    /// Background checkpointer & pruner configuration.
    #[serde(default)]
    pub pruner: PrunerConfig,
    /// Initial symbol shards registered on startup.
    #[serde(default)]
    pub symbols: Vec<SymbolConfig>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            wal: WalConfig::default(),
            pruner: PrunerConfig::default(),
            symbols: vec![],
        }
    }
}

impl AppConfig {
    /// Validates the entire configuration, returning an error if any invariant fails.
    pub fn validate(&self) -> Result<(), ConfigError> {
        self.server.validate().map_err(ConfigError::Validation)?;
        self.wal.validate().map_err(ConfigError::Validation)?;
        self.pruner.validate().map_err(ConfigError::Validation)?;

        let mut seen = HashSet::new();
        for sym in &self.symbols {
            sym.validate().map_err(ConfigError::Validation)?;
            if !seen.insert(&sym.symbol) {
                return Err(ConfigError::Validation(format!(
                    "Duplicate symbol '{}' defined in configuration",
                    sym.symbol
                )));
            }
        }
        Ok(())
    }

    /// Loads and validates configuration using the default layered sources:
    /// 1. Built-in defaults
    /// 2. Optional file (`matcher.toml`, `config.toml`, `config/matcher.toml`, or `MATCHER_CONFIG_FILE`)
    /// 3. Environment variables (prefixed `MATCHER__*` and direct `PORT`, `WAL_DIR`, etc.)
    pub fn load() -> Result<Self, ConfigError> {
        Self::load_with_path(None as Option<&Path>)
    }

    /// Loads and validates configuration with an optional explicit config file path.
    pub fn load_with_path<P: AsRef<Path>>(config_file: Option<P>) -> Result<Self, ConfigError> {
        let mut builder = config::Config::builder();

        // 1. Layer built-in defaults
        let def = Self::default();
        builder = builder
            .set_default("server.host", def.server.host)?
            .set_default("server.port", def.server.port)?
            .set_default("wal.dir", def.wal.dir.to_string_lossy().to_string())?
            .set_default("wal.sync_policy", def.wal.sync_policy.to_string())?
            .set_default("pruner.interval_secs", def.pruner.interval_secs)?
            .set_default("pruner.retain_snapshots", def.pruner.retain_snapshots as i64)?;

        for (idx, sym) in def.symbols.iter().enumerate() {
            builder = builder.set_default(format!("symbols[{idx}].symbol"), sym.symbol.clone())?;
            if let Some(price) = sym.initial_price {
                builder = builder.set_default(format!("symbols[{idx}].initial_price"), price.to_string())?;
            }
        }

        // 2. Configuration file resolution (prioritizes config.toml)
        if let Some(path) = config_file {
            builder = builder.add_source(config::File::from(path.as_ref()).required(true));
        } else if let Ok(path) = std::env::var("MATCHER_CONFIG_FILE") {
            builder = builder.add_source(config::File::with_name(&path).required(true));
        } else {
            builder = builder
                .add_source(config::File::with_name("config.toml").required(false))
                .add_source(config::File::with_name("matcher.toml").required(false))
                .add_source(config::File::with_name("config/config.toml").required(false));
        }

        // 3. Environment variables with prefix MATCHER__ (e.g. MATCHER__SERVER__PORT=50051)
        builder = builder.add_source(
            config::Environment::with_prefix("MATCHER")
                .separator("__")
                .try_parsing(true),
        );

        // 4. Legacy / Direct environment variables for backward compatibility
        if let Ok(port) = std::env::var("PORT") {
            let p: u16 = port
                .parse()
                .map_err(|_| ConfigError::Validation(format!("Invalid PORT environment variable: '{port}'")))?;
            builder = builder.set_override("server.port", p)?;
        }
        if let Ok(host) = std::env::var("HOST") {
            builder = builder.set_override("server.host", host)?;
        }
        if let Ok(wal_dir) = std::env::var("WAL_DIR") {
            builder = builder.set_override("wal.dir", wal_dir)?;
        }
        if let Ok(sync_policy) = std::env::var("WAL_SYNC_POLICY") {
            builder = builder.set_override("wal.sync_policy", sync_policy)?;
        }
        if let Ok(interval_secs) = std::env::var("WAL_PRUNE_INTERVAL_SECS") {
            let s: u64 = interval_secs.parse().map_err(|_| {
                ConfigError::Validation(format!(
                    "Invalid WAL_PRUNE_INTERVAL_SECS environment variable: '{interval_secs}'"
                ))
            })?;
            builder = builder.set_override("pruner.interval_secs", s)?;
        } else if let Ok(interval_mins) = std::env::var("WAL_PRUNE_INTERVAL_MINS") {
            let m: u64 = interval_mins.parse().map_err(|_| {
                ConfigError::Validation(format!(
                    "Invalid WAL_PRUNE_INTERVAL_MINS environment variable: '{interval_mins}'"
                ))
            })?;
            builder = builder.set_override("pruner.interval_secs", m * 60)?;
        }

        // 5. Build and deserialize into AppConfig
        let raw_config = builder.build()?;
        let app_config: AppConfig = raw_config.try_deserialize()?;

        // 6. Validate configuration invariants
        app_config.validate()?;

        Ok(app_config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_default_config_is_valid() {
        let config = AppConfig::default();
        assert!(config.validate().is_ok());
        assert_eq!(config.server.port, 50051);
        assert_eq!(config.server.host, "0.0.0.0");
        assert_eq!(config.wal.dir, PathBuf::from("data/wal"));
        assert_eq!(config.wal.sync_policy, SyncPolicy::None);
        assert_eq!(config.pruner.interval_secs, 600);
        assert_eq!(config.pruner.retain_snapshots, 2);
        assert_eq!(config.symbols.len(), 0);
    }

    #[test]
    fn test_server_config_validation() {
        let mut server = ServerConfig::default();
        assert!(server.validate().is_ok());

        server.port = 0;
        assert!(server.validate().is_err());

        server.port = 8080;
        server.host = "invalid address format".to_string();
        assert!(server.validate().is_err());

        server.host = "127.0.0.1".to_string();
        assert!(server.validate().is_ok());
        assert_eq!(server.socket_addr().unwrap(), "127.0.0.1:8080".parse().unwrap());
    }

    #[test]
    fn test_wal_config_validation() {
        let mut wal = WalConfig::default();
        assert!(wal.validate().is_ok());

        wal.dir = PathBuf::from("");
        assert!(wal.validate().is_err());
    }

    #[test]
    fn test_pruner_config_validation() {
        let mut pruner = PrunerConfig::default();
        assert!(pruner.validate().is_ok());

        pruner.interval_secs = 0;
        assert!(pruner.validate().is_err());

        pruner.interval_secs = 60;
        pruner.retain_snapshots = 0;
        assert!(pruner.validate().is_err());

        pruner.retain_snapshots = 1;
        assert!(pruner.validate().is_ok());
    }

    #[test]
    fn test_symbol_config_validation() {
        let valid_sym = SymbolConfig::new("BTC-USDT", Some(dec!(65000.0)));
        assert!(valid_sym.validate().is_ok());
        assert_eq!(
            valid_sym.initial_udecimal_price().unwrap(),
            Some(UDecimal::new(dec!(65000.0)).unwrap())
        );

        let empty_sym = SymbolConfig::new("   ", Some(dec!(100.0)));
        assert!(empty_sym.validate().is_err());

        let negative_price = SymbolConfig::new("BTC-USDT", Some(dec!(-10.0)));
        assert!(negative_price.validate().is_err());

        let zero_price = SymbolConfig::new("BTC-USDT", Some(dec!(0.0)));
        assert!(zero_price.validate().is_err());
    }

    #[test]
    fn test_duplicate_symbols_rejected() {
        let mut config = AppConfig::default();
        config.symbols.push(SymbolConfig::new("BTC-USDT", Some(dec!(10000.0))));
        config.symbols.push(SymbolConfig::new("BTC-USDT", Some(dec!(20000.0))));
        let result = config.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Duplicate symbol 'BTC-USDT'"));
    }

    #[test]
    fn test_load_from_toml_str() {
        let toml_data = r#"
            [server]
            host = "127.0.0.1"
            port = 9090

            [wal]
            dir = "/tmp/matcher_test_wal"
            sync_policy = "always"

            [pruner]
            interval_secs = 30
            retain_snapshots = 3

            [[symbols]]
            symbol = "AVAX-USDT"
            initial_price = "25.5"
        "#;

        let raw: AppConfig = config::Config::builder()
            .add_source(config::File::from_str(toml_data, config::FileFormat::Toml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();

        assert_eq!(raw.server.port, 9090);
        assert_eq!(raw.server.host, "127.0.0.1");
        assert_eq!(raw.wal.dir, PathBuf::from("/tmp/matcher_test_wal"));
        assert_eq!(raw.wal.sync_policy, SyncPolicy::Always);
        assert_eq!(raw.pruner.interval_secs, 30);
        assert_eq!(raw.pruner.retain_snapshots, 3);
        assert_eq!(raw.symbols.len(), 1);
        assert_eq!(raw.symbols[0].symbol, "AVAX-USDT");
        assert_eq!(raw.symbols[0].initial_price, Some(dec!(25.5)));
        assert!(raw.validate().is_ok());
    }

    #[test]
    fn test_load_from_toml_with_invalid_port_fails_validation() {
        let toml_data = r#"
            [server]
            port = 0
        "#;

        let raw: AppConfig = config::Config::builder()
            .add_source(config::File::from_str(toml_data, config::FileFormat::Toml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();

        let err = raw.validate().unwrap_err();
        assert!(err.to_string().contains("Server port must be non-zero"));
    }

    #[test]
    fn test_load_config_toml_file() {
        let loaded = AppConfig::load_with_path(Some("config.toml"));
        assert!(loaded.is_ok());
        let config = loaded.unwrap();
        assert_eq!(config.server.port, 50051);
        assert_eq!(config.wal.sync_policy, SyncPolicy::None);
        assert_eq!(config.symbols.len(), 3);
        assert!(config.validate().is_ok());
    }
}
