use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::wal::reader::WalReader;

/// Default background pruning interval: 10 minutes.
pub const DEFAULT_PRUNE_INTERVAL_MINS: u64 = 10;
pub const DEFAULT_PRUNE_INTERVAL_SECS: u64 = DEFAULT_PRUNE_INTERVAL_MINS * 60;

/// Environment variable names for configuring the pruning interval.
pub const ENV_PRUNE_INTERVAL_MINS: &str = "WAL_PRUNE_INTERVAL_MINS";
pub const ENV_PRUNE_INTERVAL_SECS: &str = "WAL_PRUNE_INTERVAL_SECS";

/// Configuration for the background WAL pruner.
#[derive(Debug, Clone)]
pub struct WalPrunerConfig {
    /// Root directory containing WAL segment files and snapshots.
    pub base_dir: PathBuf,
    /// Periodic cleanup interval.
    pub interval: Duration,
    /// Number of latest snapshot files to retain (default: 2 for safe fallback).
    pub retain_snapshots: usize,
}

impl WalPrunerConfig {
    /// Loads the pruning configuration, resolving the interval from environment variables
    /// (`WAL_PRUNE_INTERVAL_SECS` or `WAL_PRUNE_INTERVAL_MINS`) and defaulting to 10 minutes.
    #[must_use]
    pub fn from_env(base_dir: impl AsRef<Path>) -> Self {
        let interval_secs = std::env::var(ENV_PRUNE_INTERVAL_SECS)
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .or_else(|| {
                std::env::var(ENV_PRUNE_INTERVAL_MINS)
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok())
                    .map(|m| m * 60)
            })
            .unwrap_or(DEFAULT_PRUNE_INTERVAL_SECS);

        Self {
            base_dir: base_dir.as_ref().to_path_buf(),
            interval: Duration::from_secs(interval_secs),
            retain_snapshots: 2,
        }
    }

    /// Sets a custom pruning execution interval.
    #[must_use]
    pub fn with_interval(mut self, interval: Duration) -> Self {
        self.interval = interval;
        self
    }

    /// Sets the number of recent snapshot files to retain.
    #[must_use]
    pub fn with_retain_snapshots(mut self, count: usize) -> Self {
        self.retain_snapshots = count;
        self
    }
}

/// Statistics reported after executing a pruning cycle.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PruneStats {
    /// Number of obsolete WAL segment files removed.
    pub segments_pruned: usize,
    /// Number of old snapshot files removed.
    pub snapshots_pruned: usize,
    /// Total disk bytes reclaimed.
    pub bytes_freed: u64,
}

/// A parsed entry representing an on-disk WAL segment or snapshot file.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LogFileEntry {
    path: PathBuf,
    sequence: u64,
    size_bytes: u64,
}

/// Asynchronous background runner and pruning engine for Write-Ahead Log segments and snapshots.
#[derive(Debug, Clone)]
pub struct WalPruner {
    config: WalPrunerConfig,
}

impl WalPruner {
    /// Creates a new `WalPruner` with the given configuration.
    #[must_use]
    pub fn new(config: WalPrunerConfig) -> Self {
        Self { config }
    }

    /// Accessor for the active configuration.
    #[must_use]
    pub fn config(&self) -> &WalPrunerConfig {
        &self.config
    }

    /// Runs a single pruning pass over the root `base_dir` and any per-symbol subdirectories.
    ///
    /// # Pruning Rules
    /// 1. Discovers snapshot files (`*.snap`) and determines the highest snapshot sequence $S$.
    /// 2. If no snapshot exists, **no WAL segments are deleted** (safety invariant: all history is needed).
    /// 3. Retains the newest `retain_snapshots` files, removing older ones.
    /// 4. Discovers all WAL segment files (`*.wal`). The newest/highest-sequence segment is **always kept**
    ///    as it may be actively written.
    /// 5. Any closed WAL segment whose maximum recorded sequence $< S$ is safely deleted.
    pub fn prune_once(&self) -> Result<PruneStats, String> {
        if !self.config.base_dir.exists() {
            return Ok(PruneStats::default());
        }

        let mut total_stats = PruneStats::default();

        // Prune the root base_dir itself
        let root_stats = self.prune_directory(&self.config.base_dir)?;
        total_stats.segments_pruned += root_stats.segments_pruned;
        total_stats.snapshots_pruned += root_stats.snapshots_pruned;
        total_stats.bytes_freed += root_stats.bytes_freed;

        // Prune any per-symbol subdirectories (e.g. data/wal/BTC-USDT/)
        let entries = fs::read_dir(&self.config.base_dir)
            .map_err(|e| format!("Failed to read base dir '{}': {e}", self.config.base_dir.display()))?;

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let sub_stats = self.prune_directory(&path)?;
                total_stats.segments_pruned += sub_stats.segments_pruned;
                total_stats.snapshots_pruned += sub_stats.snapshots_pruned;
                total_stats.bytes_freed += sub_stats.bytes_freed;
            }
        }

        Ok(total_stats)
    }

    /// Prunes WAL segments and snapshots inside a specific directory.
    pub fn prune_directory(&self, dir: &Path) -> Result<PruneStats, String> {
        let mut stats = PruneStats::default();
        if !dir.exists() {
            return Ok(stats);
        }

        let mut snapshots: Vec<LogFileEntry> = Vec::new();
        let mut wal_segments: Vec<LogFileEntry> = Vec::new();

        let entries = fs::read_dir(dir)
            .map_err(|e| format!("Failed to read directory '{}': {e}", dir.display()))?;

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            let file_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();

            let metadata = match fs::metadata(&path) {
                Ok(m) => m,
                Err(_) => continue,
            };
            let size_bytes = metadata.len();

            if path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("snap")) {
                if let Some(seq) = Self::extract_sequence(file_name) {
                    snapshots.push(LogFileEntry {
                        path,
                        sequence: seq,
                        size_bytes,
                    });
                }
            } else if path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("wal")) {
                // Determine sequence from filename or fallback to reading last frame
                let seq = Self::extract_sequence(file_name).or_else(|| Self::read_max_sequence(&path));
                if let Some(seq) = seq {
                    wal_segments.push(LogFileEntry {
                        path,
                        sequence: seq,
                        size_bytes,
                    });
                }
            }
        }

        // Sort both by sequence ascending
        snapshots.sort_by_key(|e| e.sequence);
        wal_segments.sort_by_key(|e| e.sequence);

        // 1. Process Snapshots: keep the newest `retain_snapshots`, delete older ones
        let latest_snapshot_seq = snapshots.last().map(|s| s.sequence);

        if snapshots.len() > self.config.retain_snapshots {
            let to_delete_count = snapshots.len() - self.config.retain_snapshots;
            for snap in snapshots.iter().take(to_delete_count) {
                if fs::remove_file(&snap.path).is_ok() {
                    stats.snapshots_pruned += 1;
                    stats.bytes_freed += snap.size_bytes;
                }
            }
        }

        // 2. Process WAL Segments:
        // Safety check: if no snapshot exists, never delete any WAL segments!
        let Some(checkpoint_seq) = latest_snapshot_seq else {
            return Ok(stats);
        };

        // The highest-sequence segment is considered the active segment and is never deleted
        let active_segment_path = wal_segments.last().map(|s| &s.path);

        for segment in &wal_segments {
            // Never prune the active segment
            if Some(&segment.path) == active_segment_path {
                continue;
            }

            // If this segment's highest sequence is strictly before the snapshot checkpoint,
            // all its state is already safely incorporated into the snapshot and can be pruned.
            if segment.sequence < checkpoint_seq && fs::remove_file(&segment.path).is_ok() {
                stats.segments_pruned += 1;
                stats.bytes_freed += segment.size_bytes;
            }
        }

        Ok(stats)
    }

    /// Spawns the background pruner task using Tokio with a cooperative cancellation handle.
    #[must_use]
    pub fn spawn_background(self) -> PrunerHandle {
        let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
        let interval_duration = self.config.interval;

        let join_handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(interval_duration);
            // The first tick fires immediately; we skip it to wait the initial interval
            interval.tick().await;

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        if let Err(err) = self.prune_once() {
                            eprintln!("WAL Pruner error: {err}");
                        }
                    }
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            break;
                        }
                    }
                }
            }
        });

        PrunerHandle {
            shutdown_tx,
            join_handle,
        }
    }

    /// Extracts an embedded numeric sequence number from a filename.
    ///
    /// Examples:
    /// - `segment_0000000042.wal` -> `Some(42)`
    /// - `snapshot_1000.snap` -> `Some(1000)`
    /// - `BTC-USDT_00500.wal` -> `Some(500)`
    fn extract_sequence(filename: &str) -> Option<u64> {
        let stem = filename.split('.').next()?;
        // Scan for digits sequence from parts separated by '_' or '-'
        for part in stem.split(['_', '-']) {
            if let Ok(seq) = part.parse::<u64>() {
                return Some(seq);
            }
        }
        None
    }

    /// Fallback reader that inspects the highest sequence number inside a WAL file.
    fn read_max_sequence(path: &Path) -> Option<u64> {
        let mut reader = WalReader::open(path).ok()?;
        let (_, _) = reader.read_all();
        reader.last_sequence()
    }
}

/// Control handle for the asynchronous background pruning task.
#[derive(Debug)]
pub struct PrunerHandle {
    shutdown_tx: watch::Sender<bool>,
    join_handle: JoinHandle<()>,
}

impl PrunerHandle {
    /// Signals the background pruner to stop and waits for its graceful exit.
    pub async fn stop(self) {
        let _ = self.shutdown_tx.send(true);
        let _ = self.join_handle.await;
    }
}
