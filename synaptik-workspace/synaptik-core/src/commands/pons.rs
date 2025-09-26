use anyhow::Result;
use serde_json::Value;

use crate::commands::Commands;
use crate::utils::pons::{ObjectMetadata as PonsMetadata, ObjectRef as PonsObjectRef};

impl Commands {
    /// Ensure a pons namespace exists under the shared root.
    pub fn pons_create(&self, pons: &str) -> Result<()> {
        let store = self.pons_store()?;
        store.create_pons(pons)
    }

    /// Write bytes plus optional metadata into a pons/key stream.
    pub fn pons_put_object(
        &self,
        pons: &str,
        key: &str,
        data: &[u8],
        media_type: Option<&str>,
        extra: Option<Value>,
    ) -> Result<PonsObjectRef> {
        let store = self.pons_store()?;
        let (obj, _path) = store.put_object_with_meta(pons, key, data, media_type, extra)?;
        Ok(obj)
    }

    /// Read newest bytes for a pons/key.
    pub fn pons_get_latest_bytes(&self, pons: &str, key: &str) -> Result<Vec<u8>> {
        let store = self.pons_store()?;
        store.get_object_latest(pons, key)
    }

    /// Fetch newest ObjectRef for a pons/key.
    pub fn pons_get_latest_ref(&self, pons: &str, key: &str) -> Result<PonsObjectRef> {
        let store = self.pons_store()?;
        store.get_object_latest_ref(pons, key)
    }

    /// Fetch a specific version's bytes and metadata.
    pub fn pons_get_version_with_meta(
        &self,
        pons: &str,
        key: &str,
        version: &str,
    ) -> Result<(Vec<u8>, PonsMetadata)> {
        let store = self.pons_store()?;
        store.get_object_version_with_meta(pons, key, version)
    }

    /// List the latest refs under a pons namespace.
    pub fn pons_list_latest(
        &self,
        pons: &str,
        prefix: Option<&str>,
        limit: usize,
    ) -> Result<Vec<PonsObjectRef>> {
        let store = self.pons_store()?;
        store.list_latest(pons, prefix, limit)
    }
}
