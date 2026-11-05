//! Tenant-to-storage mapping: one SurrealDB database per tenant.

use anyhow::{Context, Result};
use dashmap::DashMap;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{OnceCell, watch};

use super::TenantId;
use crate::config::{Config, SurrealMode};
use crate::embeddings::EmbeddingService;
use crate::operations::OperationService;
use crate::storage::MemoryStorage;

pub type StorageFuture = Pin<Box<dyn Future<Output = Result<Arc<dyn MemoryStorage>>> + Send>>;

/// Builds the storage for one tenant. Supplied by the binary, which owns the
/// connection configuration.
pub type TenantStorageFactory = Arc<dyn Fn(TenantId) -> StorageFuture + Send + Sync>;

/// Everything a request for one tenant needs.
#[derive(Clone)]
pub struct TenantRuntime {
    pub storage: Arc<dyn MemoryStorage>,
    pub operations: OperationService,
}

pub struct TenantRegistry {
    factory: TenantStorageFactory,
    embedding_service: Arc<dyn EmbeddingService>,
    query_timeout: Duration,
    /// When present, each tenant also gets its own TTL sweep worker.
    shutdown: Option<watch::Receiver<bool>>,
    cells: DashMap<TenantId, Arc<OnceCell<TenantRuntime>>>,
}

impl TenantRegistry {
    pub fn new(
        factory: TenantStorageFactory,
        embedding_service: Arc<dyn EmbeddingService>,
        query_timeout: Duration,
        shutdown: Option<watch::Receiver<bool>>,
    ) -> Self {
        Self {
            factory,
            embedding_service,
            query_timeout,
            shutdown,
            cells: DashMap::new(),
        }
    }

    /// The runtime for `tenant`, created on first use. Concurrent first
    /// requests share one initialisation; a failed one is retried by the next
    /// request.
    pub async fn runtime(&self, tenant: &TenantId) -> Result<TenantRuntime> {
        let cell = Arc::clone(
            self.cells
                .entry(tenant.clone())
                .or_insert_with(|| Arc::new(OnceCell::new()))
                .value(),
        );
        cell.get_or_try_init(|| self.create(tenant))
            .await
            .cloned()
            .with_context(|| format!("tenant '{tenant}' storage is unavailable"))
    }

    async fn create(&self, tenant: &TenantId) -> Result<TenantRuntime> {
        let storage = (self.factory)(tenant.clone()).await?;
        let operations = OperationService::start_with_query_timeout(
            Arc::clone(&storage),
            Arc::clone(&self.embedding_service),
            self.query_timeout,
        );
        if let Some(shutdown) = &self.shutdown {
            let worker_storage = Arc::clone(&storage);
            let worker_shutdown = shutdown.clone();
            let tenant = tenant.clone();
            tokio::spawn(async move {
                if let Err(error) =
                    crate::workers::ttl::run_ttl_worker(worker_storage, worker_shutdown).await
                {
                    tracing::error!(%tenant, %error, "tenant TTL worker stopped");
                }
            });
        }
        tracing::info!(%tenant, "tenant storage ready");
        Ok(TenantRuntime {
            storage,
            operations,
        })
    }
}

/// The configuration of `tenant`'s own database, derived from the base
/// configuration. The tenant id is already restricted to `[a-z0-9_]`, so it is
/// safe in a namespace and in a path.
pub fn tenant_config(base: &Config, tenant: &TenantId) -> Result<Config> {
    let mut config = base.clone();
    match base.surreal_mode {
        SurrealMode::Server => {
            config.surreal_namespace = format!("{}_{}", base.surreal_namespace, tenant);
        }
        SurrealMode::Embedded => {
            let root = base
                .embedded_path
                .as_deref()
                .context("tenant isolation in embedded mode needs SURREAL_PATH")?;
            config.embedded_path = Some(
                Path::new(root)
                    .join("tenants")
                    .join(tenant.as_str())
                    .to_string_lossy()
                    .into_owned(),
            );
        }
    }
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base(mode: &str, path: Option<&str>) -> Config {
        let mut config: Config = serde_json::from_value(serde_json::json!({
            "surreal_mode": mode,
            "surreal_endpoint": null,
            "surreal_namespace": "memory",
            "surreal_database": "main",
            "surreal_username": null,
            "surreal_password": null,
            "embedded_path": path,
            "embedding_provider": {"provider": "openai", "api_key": "k", "model": "m"},
            "local_embedding_backend": "candle",
            "local_embedding_executor": null,
            "local_embedding_model_revision": "r",
            "local_embedding_dimensions": 8,
            "embedding_warmup": false
        }))
        .expect("test config deserializes");
        config.surreal_database = "main".into();
        config
    }

    fn tenant(raw: &str) -> TenantId {
        TenantId::parse(raw).unwrap()
    }

    #[test]
    fn server_mode_gives_each_tenant_its_own_namespace() {
        let config = base("server", None);
        let acme = tenant_config(&config, &tenant("acme")).unwrap();
        let globex = tenant_config(&config, &tenant("globex")).unwrap();
        assert_eq!(acme.surreal_namespace, "memory_acme");
        assert_eq!(globex.surreal_namespace, "memory_globex");
        assert_eq!(acme.surreal_database, "main");
    }

    #[test]
    fn embedded_mode_gives_each_tenant_its_own_directory() {
        let config = base("embedded", Some("/data/memory"));
        let acme = tenant_config(&config, &tenant("acme")).unwrap();
        assert_eq!(
            acme.embedded_path.as_deref(),
            Some("/data/memory/tenants/acme")
        );
    }

    #[test]
    fn embedded_mode_without_a_path_cannot_isolate_tenants() {
        assert!(tenant_config(&base("embedded", None), &tenant("acme")).is_err());
    }

    #[test]
    fn tenant_ids_that_could_escape_a_namespace_or_path_are_refused() {
        for bad in ["", "../x", "a/b", "A", "a-b", "a b", "a.b", &"x".repeat(49)] {
            assert!(TenantId::parse(bad).is_err(), "{bad:?}");
        }
        assert!(TenantId::parse("acme_2").is_ok());
    }
}
