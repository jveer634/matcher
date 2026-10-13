use std::net::SocketAddr;
use std::sync::Arc;

use crate::engine::ShardedEngine;
use crate::grpc::{MatcherServiceImpl, MatcherServiceServer};
use crate::wal::pruner::{PrunerHandle, WalPruner};

/// Core Application Server managing the life cycle and orchestration of all services
/// (e.g. matching engine shards, background WAL pruner, and future services).
#[derive(Debug)]
pub struct Server {
    /// Local socket address the gRPC server binds to.
    addr: SocketAddr,
    /// Multi-symbol horizontal sharded matching engine.
    engine: Arc<ShardedEngine>,
    /// Background Write-Ahead Log checkpointer and pruner.
    pruner: WalPruner,
    /// Asynchronous join handle for the running background pruner.
    pruner_handle: Option<PrunerHandle>,
}

impl Server {
    /// Constructs a new `Server` with attached engine and background services.
    #[must_use]
    pub fn new(
        addr: SocketAddr,
        engine: Arc<ShardedEngine>,
        pruner: WalPruner,
        pruner_handle: Option<PrunerHandle>,
    ) -> Self {
        Self {
            addr,
            engine,
            pruner,
            pruner_handle,
        }
    }

    /// Accessor for the underlying sharded matching engine.
    #[must_use]
    pub fn engine(&self) -> &Arc<ShardedEngine> {
        &self.engine
    }

    /// Accessor for the WAL pruner service.
    #[must_use]
    pub fn pruner(&self) -> &WalPruner {
        &self.pruner
    }

    /// Accessor for the bound socket address.
    #[must_use]
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Sets or updates the background pruner task handle.
    pub fn set_pruner_handle(&mut self, handle: PrunerHandle) {
        self.pruner_handle = Some(handle);
    }

    /// Runs the gRPC server until shutdown.
    /// Gracefully terminates attached background tasks upon exit.
    pub async fn serve(mut self) -> Result<(), Box<dyn std::error::Error>> {
        let service = MatcherServiceImpl::new(Arc::clone(&self.engine));

        tracing::info!(
            addr = %self.addr,
            symbols = ?self.engine.list_symbols(),
            pruner_interval_secs = self.pruner.config().interval.as_secs(),
            pruner_retain_snapshots = self.pruner.config().retain_snapshots,
            "Matcher High-Performance gRPC Service listening"
        );

        let serve_res = tonic::transport::Server::builder()
            .add_service(MatcherServiceServer::new(service))
            .serve_with_shutdown(self.addr, async {
                if let Err(err) = tokio::signal::ctrl_c().await {
                    tracing::error!(error = %err, "Failed to install Ctrl+C signal handler");
                } else {
                    tracing::info!("Shutdown signal received. Gracefully stopping services...");
                }
            })
            .await;

        // 1. Gracefully stop attached background pruner on shutdown
        if let Some(handle) = self.pruner_handle.take() {
            tracing::info!("Stopping background WAL pruner task...");
            handle.stop().await;
        }

        // 2. Run parallel shutdown agents across all symbol shards (final snapshot checkpoint + WAL buffer flush)
        tracing::info!("Executing parallel shutdown agents (final snapshot & WAL flush)...");
        if let Err(err) = self.engine.shutdown_parallel(true).await {
            tracing::error!(error = %err, "Error during parallel shutdown agents");
        }

        tracing::info!("Matcher service stopped cleanly");

        serve_res.map_err(Into::into)
    }
}
