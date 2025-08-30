//! services/lobes.rs
//!
//! Implements a lightweight content-addressed object store for Synaptik.
//! - Each "lobe" is a logical namespace (like a bookshelf).
//! - Each "key" is a file path under that lobe (like a book title).
//! - Each key stores immutable, versioned objects under `<root>/objects/<lobe>/<key>/versions/`.
//! - A `LATEST` pointer file tracks the newest version for fast retrieval.
//!
//! This module provides atomic write, version scanning, and safe key sanitation,
//! without involving the SQLite cache or DAG layers. It is purely filesystem-backed.

use anyhow::{Context, Result};
use blake3;
use chrono::Utc;
use std::{
    fs,
    ffi::OsStr,
    io::Write,
    path::{Component, Path, PathBuf},
};

/// Filesystem-backed store for versioned objects grouped into "lobes".
/// Root is typically `.cogniv/objects`.
pub struct LobeStore {
    root: PathBuf, // e.g., .cogniv/objects
}

impl LobeStore {
    /// Open or initialize a lobe store at the given root.
    ///
    /// Creates the `objects/` directory if missing. Does not use any DB.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        fs::create_dir_all(root.join("objects"))?;
        Ok(Self { root })
    }

    /// Ensure a given lobe exists (idempotent).
    ///
    /// Example: `.cogniv/objects/notes/`
    pub fn create_lobe(&self, name: &str) -> Result<()> {
        fs::create_dir_all(self.root.join("objects").join(name))?;
        Ok(())
    }

    /// Store a new object version under a `(lobe, key)`.
    ///
    /// - Writes to `<root>/objects/<lobe>/<key>/versions/<ts>-<hash12>.bin`.
    /// - Updates the `LATEST` pointer atomically.
    /// - Returns `(version_id, etag, absolute_path)`.
    pub fn put_object(&self, lobe: &str, key: &str, data: &[u8]) -> Result<(String, String, PathBuf)> {
        let lobe = sanitize_key(lobe)?;
        let key = sanitize_key(key)?;

        let dir = self.root.join("objects").join(&lobe).join(&key).join("versions");
        fs::create_dir_all(&dir)?;

        let etag = blake3::hash(data).to_hex().to_string();
        let ts_ms = Utc::now().timestamp_millis(); // sortable timestamp
        let version_id = format!("{}-{}", ts_ms, &etag[..12]);

        let file_path = dir.join(format!("{version_id}.bin"));
        write_atomic(&file_path, data)?;

        // Update "LATEST" pointer atomically
        let latest_path = self.root.join("objects").join(&lobe).join(&key).join("LATEST");
        write_atomic(latest_path.as_path(), version_id.as_bytes())?;

        Ok((version_id, etag, file_path))
    }

    /// Read the latest version of an object.
    ///
    /// - Uses the `LATEST` pointer if present.
    /// - Falls back to scanning the `versions/` directory.
    pub fn get_object_latest(&self, lobe: &str, key: &str) -> Result<Vec<u8>> {
        let lobe = sanitize_key(lobe)?;
        let key = sanitize_key(key)?;
        let base = self.root.join("objects").join(&lobe).join(&key);

        // Try LATEST pointer first
        let latest_path = base.join("LATEST");
        let ver = match fs::read_to_string(&latest_path) {
            Ok(s) => s.trim().to_string(),
            Err(_) => self.scan_latest_version(&base)?,
        };
        self.get_object_version(&lobe, &key, &ver)
    }

    /// Read a specific version of an object.
    ///
    /// `version_id` must match a file under `versions/<version_id>.bin`.
    pub fn get_object_version(&self, lobe: &str, key: &str, version_id: &str) -> Result<Vec<u8>> {
        let lobe = sanitize_key(lobe)?;
        let key = sanitize_key(key)?;
        let path = self
            .root
            .join("objects")
            .join(&lobe)
            .join(&key)
            .join("versions")
            .join(format!("{version_id}.bin"));
        Ok(fs::read(&path).with_context(|| format!("read {:?}", path))?)
    }

    /// List the latest version of keys under a given lobe.
    ///
    /// - Traverses recursively from the lobe directory.
    /// - Uses the `LATEST` pointer or fallback scanning.
    /// - Returns up to `limit` entries of `(key, version_id, size_bytes)`.
    pub fn list_latest(&self, lobe: &str, prefix: Option<&str>, limit: usize) -> Result<Vec<(String, String, u64)>> {
        let lobe = sanitize_key(lobe)?;
        let pref = prefix.unwrap_or("").trim_matches('/');

        let lobe_dir = self.root.join("objects").join(&lobe);
        if !lobe_dir.exists() {
            return Ok(Vec::new());
        }

        let mut out = Vec::new();
        let mut stack = vec![lobe_dir.clone()];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir)? {
                let entry = entry?;
                let path = entry.path();

                if path.is_dir() && path.file_name().map(|s| s != OsStr::new("versions")).unwrap_or(false) {
                    let versions = path.join("versions");
                    if versions.is_dir() {
                        // derive key relative to lobe root
                        let key_rel = path.strip_prefix(&lobe_dir).unwrap().to_string_lossy().replace('\\', "/");
                        if !key_rel.starts_with(pref) { continue; }

                        // latest version via pointer or scan
                        let latest = match fs::read_to_string(path.join("LATEST")) {
                            Ok(s) => s.trim().to_string(),
                            Err(_) => self.scan_latest_version(&path)?,
                        };

                        // file size of latest object
                        let fpath = versions.join(format!("{latest}.bin"));
                        let sz = fs::metadata(&fpath)?.len();

                        out.push((key_rel, latest, sz));
                        if out.len() >= limit { return Ok(out); }
                    } else {
                        stack.push(path);
                    }
                }
            }
        }
        Ok(out)
    }

    /// Internal: scan the `versions/` directory to find the latest version.
    ///
    /// Picks the lexicographically max filename (timestamps ensure ordering).
    fn scan_latest_version(&self, key_dir: &Path) -> Result<String> {
        let versions = key_dir.join("versions");
        let mut best: Option<String> = None;
        if versions.is_dir() {
            for entry in fs::read_dir(&versions)? {
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().into_owned();
                if let Some(stripped) = name.strip_suffix(".bin") {
                    if best.as_ref().map(|b| stripped > b.as_str()).unwrap_or(true) {
                        best = Some(stripped.to_string());
                    }
                }
            }
        }
        best.ok_or_else(|| anyhow::anyhow!("no versions found under {:?}", versions))
    }
}

// ---------- helpers ----------

/// Atomically write bytes to a file.
/// Uses a `.tmp` file then renames for crash-safety.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Sanitize keys and lobes before use.
/// - Forbids `..` parent components.
/// - Normalizes separators to `/`.
/// - Allows nested keys like `session/2025-08-26/conv.txt`.
fn sanitize_key(key: &str) -> Result<String> {
    let p = Path::new(key);
    for c in p.components() {
        if matches!(c, Component::ParentDir) {
            anyhow::bail!("parent paths not allowed");
        }
    }
    let s = key.trim().trim_matches('/').replace('\\', "/");
    if s.is_empty() { anyhow::bail!("empty key") }
    Ok(s)
}
