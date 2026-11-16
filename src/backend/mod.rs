mod rbw;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::runtime::Runtime;

pub use rbw::RbwProvider;

use crate::model::{ItemDetails, ItemMeta};

#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("rbw is not installed or not on PATH")]
    NotInstalled,
    #[error("rbw is not configured yet (run `rbw config set email <email>`)")]
    NotInitialized,
    #[error("the vault is locked")]
    Locked,
    #[error("authentication failed")]
    AuthFailed,
    #[error("{0}")]
    NotFound(String),
    #[error("rbw timed out while {0}")]
    Timeout(String),
    #[error("could not parse rbw output: {0}")]
    InvalidJson(String),
    #[error("{0}")]
    CommandFailed(String),
}

impl VaultError {
    pub fn user_message(&self) -> String {
        self.to_string()
    }

    pub fn is_locked(&self) -> bool {
        matches!(self, Self::Locked)
    }
}

pub type Result<T> = std::result::Result<T, VaultError>;

#[async_trait]
pub trait VaultProvider: Send + Sync {
    async fn is_installed(&self) -> Result<()>;
    async fn is_unlocked(&self) -> Result<bool>;
    async fn list_items(&self) -> Result<Vec<ItemMeta>>;
    async fn get_item(&self, id: &str) -> Result<ItemDetails>;
    async fn get_totp(&self, id: &str) -> Result<String>;
    async fn lock(&self) -> Result<()>;
    async fn unlock(&self) -> Result<()>;
    async fn sync(&self) -> Result<()>;
}

#[derive(Clone)]
pub struct Backend {
    inner: Arc<BackendInner>,
}

struct BackendInner {
    runtime: Runtime,
    provider: Arc<dyn VaultProvider>,
}

impl Backend {
    pub fn new(rbw_path: String) -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("vault-backend")
            .build()
            .expect("failed to start backend runtime");
        Self {
            inner: Arc::new(BackendInner {
                runtime,
                provider: Arc::new(RbwProvider::new(rbw_path)),
            }),
        }
    }

    async fn call<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(Arc<dyn VaultProvider>) -> futures_lite_future::BoxFuture<'static, Result<T>>
            + Send
            + 'static,
    {
        let provider = self.inner.provider.clone();
        self.inner
            .runtime
            .spawn(async move { f(provider).await })
            .await
            .map_err(|err| VaultError::CommandFailed(format!("backend task failed: {err}")))?
    }

    pub async fn is_installed(&self) -> Result<()> {
        self.call(|p| Box::pin(async move { p.is_installed().await }))
            .await
    }

    pub async fn is_unlocked(&self) -> Result<bool> {
        self.call(|p| Box::pin(async move { p.is_unlocked().await }))
            .await
    }

    pub async fn list_items(&self) -> Result<Vec<ItemMeta>> {
        self.call(|p| Box::pin(async move { p.list_items().await }))
            .await
    }

    pub async fn get_item(&self, id: String) -> Result<ItemDetails> {
        self.call(move |p| Box::pin(async move { p.get_item(&id).await }))
            .await
    }

    pub async fn get_totp(&self, id: String) -> Result<String> {
        self.call(move |p| Box::pin(async move { p.get_totp(&id).await }))
            .await
    }

    pub async fn lock(&self) -> Result<()> {
        self.call(|p| Box::pin(async move { p.lock().await })).await
    }

    pub async fn unlock(&self) -> Result<()> {
        self.call(|p| Box::pin(async move { p.unlock().await }))
            .await
    }

    pub async fn sync(&self) -> Result<()> {
        self.call(|p| Box::pin(async move { p.sync().await })).await
    }
}

/// Tiny alias so `call` does not depend on a futures crate at the call site.
mod futures_lite_future {
    pub type BoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;
}

pub const LIST_TIMEOUT: Duration = Duration::from_secs(30);
pub const DETAIL_TIMEOUT: Duration = Duration::from_secs(20);
pub const UNLOCK_TIMEOUT: Duration = Duration::from_secs(180);
pub const SYNC_TIMEOUT: Duration = Duration::from_secs(300);
