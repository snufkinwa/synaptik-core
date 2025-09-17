//! services/pons.rs
//!
//! A filesystem-backed, versioned object store for Synaptik.
//!
//! # Concept
//! - A **pons** is a logical namespace (like a bookshelf) for sensor or media payloads.
//! - A **key** is a path under that pons (like a book title).
//! - Each key has immutable, versioned objects under:
//!     `<root>/objects/<pons>/<key>/versions/<ts>-<hash12>.bin`
//! - A `LATEST` pointer file tracks the newest version for O(1) retrieval.
//! - A JSON sidecar `<version>.json` stores media type, size, etag, and extra metadata.
//!
//! # Purpose
//! - Handles large, non-DB artifacts (photos, audio, graphs, embeddings).
//! - Provides atomic write, safe key sanitation, and version scanning.
//! - No SQLite, no DAG — it is purely filesystem-backed.
//!
//! # Background
//! Pons grew out of robotics experiments where OpenCV pipelines emitted rapid
//! bursts of camera frames, depth maps, and feature graphs. Those artifacts
//! were too large and binary for SQLite, yet every version needed to survive a
//! crash and remain replayable. The pons store provides that bridge: sensor
//! streams land here immutably, while the memory DAG keeps lightweight
//! pointers so experiences can be reconstructed frame by frame.
//!
//! # Integration
//! - The DAG should store an `ObjectRef` (pons, key, version, etag, size).
//! - The Archivist (or any agent) can materialize bytes or metadata
//!   on demand from an `ObjectRef`.
//! - Use pons when data is too big, too binary, or too irregular for the
//!   SQLite hot cache or append-only logbook.

use anyhow::{Context, Result};
use blake3;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    ffi::OsStr,
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
};

const OBJECTS_DIR: &str = "objects";
const VERSIONS_DIR: &str = "versions";
const LATEST_FILE: &str = "LATEST";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ObjectRef {
    pub pons: String,
    pub key: String,
    pub version: String,
    pub etag: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ObjectMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ObjectSidecar {
    etag: String,
    size_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    media_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    extra: Option<Value>,
}

/// Filesystem-backed store for versioned objects grouped into "pons".
/// Root is typically `.cogniv/objects`.
pub struct PonsStore {
    root: PathBuf, // e.g., .cogniv
}

impl PonsStore {
    /// Open or initialize a pons store at the given root.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        fs::create_dir_all(root.join(OBJECTS_DIR))?;
        Ok(Self { root })
    }

    /// Ensure a given pons namespace exists (idempotent).
    pub fn create_pons(&self, name: &str) -> Result<()> {
        let pons = sanitize_key(name)?;
        fs::create_dir_all(self.pons_dir(&pons))?;
        Ok(())
    }

    /// Legacy helper that writes bytes without metadata.
    /// Prefer [`put_object_with_meta`] to capture media type and extras.
    pub fn put_object(
        &self,
        pons: &str,
        key: &str,
        data: &[u8],
    ) -> Result<(String, String, PathBuf)> {
        let (obj_ref, path) = self.put_object_with_meta(pons, key, data, None, None::<Value>)?;
        Ok((obj_ref.version, obj_ref.etag, path))
    }

    /// Store a new object version with optional media type and metadata.
    pub fn put_object_with_meta(
        &self,
        pons: &str,
        key: &str,
        data: &[u8],
        media_type: Option<&str>,
        extra: Option<Value>,
    ) -> Result<(ObjectRef, PathBuf)> {
        let (pons, key) = normalize_pair(pons, key)?;
        let versions_dir = self.versions_dir(&pons, &key);
        fs::create_dir_all(&versions_dir)?;

        let etag = blake3::hash(data).to_hex().to_string();
        let ts_ms = Utc::now().timestamp_millis();
        let version_id = format!("{}-{}", ts_ms, &etag[..12]);
        let data_path = data_path(&versions_dir, &version_id);
        write_atomic(&data_path, data)?;

        let sidecar = ObjectSidecar {
            etag: etag.clone(),
            size_bytes: data.len() as u64,
            media_type: media_type.map(|s| s.to_string()),
            extra,
        };
        let sidecar_path = sidecar_path(&versions_dir, &version_id);
        let sidecar_bytes = serde_json::to_vec_pretty(&sidecar)?;
        write_atomic(&sidecar_path, &sidecar_bytes)?;

        let latest_path = self.key_dir(&pons, &key).join(LATEST_FILE);
        write_atomic(latest_path.as_path(), version_id.as_bytes())?;

        let object_ref = ObjectRef {
            pons,
            key,
            version: version_id,
            etag,
            size_bytes: sidecar.size_bytes,
        };
        Ok((object_ref, data_path))
    }

    /// Read the latest version of an object as raw bytes.
    pub fn get_object_latest(&self, pons: &str, key: &str) -> Result<Vec<u8>> {
        let (pons, key) = normalize_pair(pons, key)?;
        let latest = self.latest_version(&pons, &key)?;
        self.read_version_bytes(&pons, &key, &latest)
    }

    /// Retrieve the latest [`ObjectRef`] for a `(pons, key)` pair.
    pub fn get_object_latest_ref(&self, pons: &str, key: &str) -> Result<ObjectRef> {
        let (pons, key) = normalize_pair(pons, key)?;
        let version_id = self.latest_version(&pons, &key)?;
        self.get_object_ref(&pons, &key, &version_id)
    }

    /// Read a specific object version as raw bytes.
    pub fn get_object_version(&self, pons: &str, key: &str, version_id: &str) -> Result<Vec<u8>> {
        let (pons, key) = normalize_pair(pons, key)?;
        self.read_version_bytes(&pons, &key, version_id)
    }

    /// Read a specific version, returning bytes alongside metadata.
    pub fn get_object_version_with_meta(
        &self,
        pons: &str,
        key: &str,
        version_id: &str,
    ) -> Result<(Vec<u8>, ObjectMetadata)> {
        let bytes = self.get_object_version(pons, key, version_id)?;
        let meta = self.get_object_metadata(pons, key, version_id)?;
        Ok((bytes, meta))
    }

    /// Load metadata for a specific version. Returns defaults if sidecar missing.
    pub fn get_object_metadata(
        &self,
        pons: &str,
        key: &str,
        version_id: &str,
    ) -> Result<ObjectMetadata> {
        let (pons, key) = normalize_pair(pons, key)?;
        let versions_dir = self.versions_dir(&pons, &key);
        let sidecar = load_sidecar(&versions_dir, version_id)?;
        Ok(match sidecar {
            Some(s) => ObjectMetadata {
                media_type: s.media_type,
                extra: s.extra,
            },
            None => ObjectMetadata::default(),
        })
    }

    /// Construct an [`ObjectRef`] for an existing version, recomputing metadata if needed.
    pub fn get_object_ref(&self, pons: &str, key: &str, version_id: &str) -> Result<ObjectRef> {
        let (pons, key) = normalize_pair(pons, key)?;
        let versions_dir = self.versions_dir(&pons, &key);
        let sidecar = load_sidecar(&versions_dir, version_id)?;
        let data_path = data_path(&versions_dir, version_id);

        let (etag, size_bytes) = if let Some(ref s) = sidecar {
            (s.etag.clone(), s.size_bytes)
        } else {
            let bytes = fs::read(&data_path)?;
            (
                blake3::hash(&bytes).to_hex().to_string(),
                bytes.len() as u64,
            )
        };

        Ok(ObjectRef {
            pons,
            key,
            version: version_id.to_string(),
            etag,
            size_bytes,
        })
    }

    /// List the latest version for keys under a pons, returning at most `limit` refs.
    ///
    /// Deterministic and scalable(ish): traverses the directory tree in lexicographic
    /// order so results are stable between runs and short-circuits when `limit` is reached.
    /// This delegates to `list_latest_page` with no cursor.
    pub fn list_latest(
        &self,
        pons: &str,
        prefix: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ObjectRef>> {
        self.list_latest_page(pons, prefix, None, limit)
    }

    /// Deterministic, paginated listing of latest object refs for keys under a `pons`.
    ///
    /// - Traversal is lexicographically ordered by key path to ensure stable output.
    /// - `prefix` filters by normalized key prefix.
    /// - Cursor format (`start_after`): expects a normalized key path (same normalization
    ///   as `prefix` — `trim`, `sanitize_key`, and forward slashes). The cursor is treated
    ///   as exclusive: results include keys strictly greater than the cursor, based on
    ///   lexicographic comparison of the normalized path. Callers should pass keys produced
    ///   by this system or normalize with the same rules before calling.
    /// - `limit` is the maximum number of entries returned; traversal short-circuits once met.
    /// - When `prefix` is provided, only keys with the normalized path starting with that
    ///   prefix are considered for both cursor comparison and output.
    ///
    /// Notes and future work:
    /// - For very large stores, walking the filesystem is still O(n). A persistent
    ///   B-Tree or on-disk index keyed by `<key_rel>` → `<latest_version>` would
    ///   provide O(log n) seek plus O(k) page reads. Hook here to swap in such an
    ///   index when available.
    pub fn list_latest_page(
        &self,
        pons: &str,
        prefix: Option<&str>,
        start_after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ObjectRef>> {
        use std::collections::BTreeSet;

        let pons = sanitize_key(pons)?;
        let pref_norm = if let Some(raw) = prefix {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                String::new()
            } else {
                sanitize_key(trimmed)?
            }
        } else {
            String::new()
        };
        let cursor_norm = if let Some(cur) = start_after {
            let trimmed = cur.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(sanitize_key(trimmed)?)
            }
        } else {
            None
        };

        let pons_dir = self.pons_dir(&pons);
        if !pons_dir.exists() || limit == 0 {
            return Ok(Vec::new());
        }

        // Lexicographically ordered frontier for deterministic traversal.
        let mut frontier: BTreeSet<PathBuf> = BTreeSet::new();
        frontier.insert(pons_dir.clone());

        let mut out = Vec::with_capacity(limit.min(128));
        while let Some(dir) = frontier.iter().next().cloned() {
            frontier.remove(&dir);

            // Collect child directories and insert into frontier (BTreeSet keeps them sorted).
            for entry in fs::read_dir(&dir)? {
                let entry = entry?;
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }

                // Skip the VERSIONS leaf directory itself.
                if path
                    .file_name()
                    .map(|s| s == OsStr::new(VERSIONS_DIR))
                    .unwrap_or(false)
                {
                    continue;
                }

                let versions = path.join(VERSIONS_DIR);
                if versions.is_dir() {
                    let key_rel = path
                        .strip_prefix(&pons_dir)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/");

                    if !pref_norm.is_empty() && !key_rel.starts_with(&pref_norm) {
                        continue;
                    }
                    if let Some(cur) = &cursor_norm {
                        if key_rel <= *cur {
                            // Not past the cursor yet; skip.
                            continue;
                        }
                    }

                    let latest = match fs::read_to_string(path.join(LATEST_FILE)) {
                        Ok(s) => s.trim().to_string(),
                        Err(_) => self.scan_latest_version(&versions)?,
                    };
                    let obj_ref = self.get_object_ref(&pons, &key_rel, &latest)?;
                    out.push(obj_ref);
                    if out.len() >= limit {
                        return Ok(out);
                    }
                } else {
                    frontier.insert(path);
                }
            }
        }

        Ok(out)
    }

    fn latest_version(&self, pons: &str, key: &str) -> Result<String> {
        let key_dir = self.key_dir(pons, key);
        let pointer = key_dir.join(LATEST_FILE);
        if let Ok(s) = fs::read_to_string(&pointer) {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                return Ok(trimmed.to_string());
            }
        }
        self.scan_latest_version(&key_dir.join(VERSIONS_DIR))
    }

    fn read_version_bytes(&self, pons: &str, key: &str, version_id: &str) -> Result<Vec<u8>> {
        let versions_dir = self.versions_dir(pons, key);
        let path = data_path(&versions_dir, version_id);
        Ok(fs::read(&path).with_context(|| format!("read {:?}", path))?)
    }

    /// Picks the lexicographically max filename (timestamps ensure ordering).
    fn scan_latest_version(&self, versions_dir: &Path) -> Result<String> {
        let mut best: Option<String> = None;
        if versions_dir.is_dir() {
            for entry in fs::read_dir(versions_dir)? {
                let entry = entry?;
                if !entry.file_type()?.is_file() {
                    continue;
                }
                let file_name = entry.file_name();
                let name = match file_name.to_str() {
                    Some(n) => n,
                    None => continue,
                };
                if let Some(stem) = name.strip_suffix(".bin") {
                    if best.as_ref().map(|b| stem > b.as_str()).unwrap_or(true) {
                        best = Some(stem.to_string());
                    }
                }
            }
        }
        best.ok_or_else(|| anyhow::anyhow!("no versions found under {:?}", versions_dir))
    }

    fn pons_dir(&self, pons: &str) -> PathBuf {
        self.root.join(OBJECTS_DIR).join(pons)
    }

    fn key_dir(&self, pons: &str, key: &str) -> PathBuf {
        self.pons_dir(pons).join(key)
    }

    fn versions_dir(&self, pons: &str, key: &str) -> PathBuf {
        self.key_dir(pons, key).join(VERSIONS_DIR)
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

fn data_path(versions_dir: &Path, version: &str) -> PathBuf {
    versions_dir.join(format!("{version}.bin"))
}

fn sidecar_path(versions_dir: &Path, version: &str) -> PathBuf {
    versions_dir.join(format!("{version}.json"))
}

fn load_sidecar(versions_dir: &Path, version: &str) -> Result<Option<ObjectSidecar>> {
    let path = sidecar_path(versions_dir, version);
    if !path.is_file() {
        return Ok(None);
    }
    let bytes = fs::read(&path).with_context(|| format!("read sidecar {:?}", path))?;
    let sidecar =
        serde_json::from_slice(&bytes).with_context(|| format!("parse sidecar {:?}", path))?;
    Ok(Some(sidecar))
}

fn normalize_pair(pons: &str, key: &str) -> Result<(String, String)> {
    Ok((sanitize_key(pons)?, sanitize_key(key)?))
}

/// Sanitize keys and pons before use.
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
    if s.is_empty() {
        anyhow::bail!("empty key")
    }
    Ok(s)
}
