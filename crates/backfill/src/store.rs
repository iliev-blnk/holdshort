//! Blocking wrapper over `object_store`: R2 (S3 API), a local dir, or memory.

use std::sync::Arc;

use anyhow::Context;
use futures::TryStreamExt;
use object_store::aws::AmazonS3Builder;
use object_store::local::LocalFileSystem;
use object_store::path::Path;
use object_store::{ObjectStore, ObjectStoreExt, PutPayload};
use tokio::runtime::Runtime;

pub const BUCKET: &str = "holdshort";

/// Object storage with blocking calls; owns a small tokio runtime because
/// `object_store` is async while the rest of the CLI is plain blocking code.
pub struct Store {
    inner: Arc<dyn ObjectStore>,
    rt: Runtime,
}

fn env(name: &str) -> anyhow::Result<String> {
    std::env::var(name).with_context(|| format!("missing env var {name}"))
}

impl Store {
    fn new(inner: Arc<dyn ObjectStore>) -> Self {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        Self { inner, rt }
    }

    /// Cloudflare R2 from `R2_ACCOUNT_ID`, `R2_ACCESS_KEY_ID`, `R2_SECRET_ACCESS_KEY`.
    pub fn from_env() -> anyhow::Result<Self> {
        let s3 = AmazonS3Builder::new()
            .with_endpoint(format!(
                "https://{}.r2.cloudflarestorage.com",
                env("R2_ACCOUNT_ID")?
            ))
            .with_region("auto")
            .with_bucket_name(BUCKET)
            .with_access_key_id(env("R2_ACCESS_KEY_ID")?)
            .with_secret_access_key(env("R2_SECRET_ACCESS_KEY")?)
            .build()?;
        Ok(Self::new(Arc::new(s3)))
    }

    /// Keys as files under `root` (for `--dry-run`).
    pub fn local(root: &str) -> anyhow::Result<Self> {
        std::fs::create_dir_all(root)?;
        Ok(Self::new(Arc::new(LocalFileSystem::new_with_prefix(root)?)))
    }

    #[cfg(test)]
    pub fn memory() -> Self {
        Self::new(Arc::new(object_store::memory::InMemory::new()))
    }

    pub fn exists(&self, key: &str) -> anyhow::Result<bool> {
        match self.rt.block_on(self.inner.head(&Path::from(key))) {
            Ok(_) => Ok(true),
            Err(object_store::Error::NotFound { .. }) => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    pub fn put(&self, key: &str, data: Vec<u8>) -> anyhow::Result<()> {
        self.rt
            .block_on(self.inner.put(&Path::from(key), PutPayload::from(data)))?;
        Ok(())
    }

    pub fn get(&self, key: &str) -> anyhow::Result<Vec<u8>> {
        let bytes = self
            .rt
            .block_on(async { self.inner.get(&Path::from(key)).await?.bytes().await })?;
        Ok(bytes.to_vec())
    }

    /// Keys under a prefix (e.g. `raw/live/2026-10-08/`).
    pub fn list(&self, prefix: &str) -> anyhow::Result<Vec<String>> {
        let metas: Vec<_> = self.rt.block_on(
            self.inner
                .list(Some(&Path::from(prefix.trim_end_matches('/'))))
                .try_collect::<Vec<_>>(),
        )?;
        Ok(metas.into_iter().map(|m| m.location.to_string()).collect())
    }
}
