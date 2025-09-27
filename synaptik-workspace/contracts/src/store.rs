use crate::api::{uuidv7, CapsAnnot, CapsId, Denied, Purpose, Verdict};
use crate::capsule::SimCapsule;
use anyhow::{Context, Result};
use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Handle returned from ingest
#[derive(Debug, Clone)]
pub struct CapsHandle {
    pub id: CapsId,
    pub hash: String,
}

#[derive(Debug, Clone)]
pub struct ContractsStore {
    root: PathBuf,
}

impl ContractsStore {
    pub fn new<P: AsRef<Path>>(root: P) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(root.join("capsules"))?;
        fs::create_dir_all(root.join("annotations"))?;
        fs::create_dir_all(root.join("handles"))?;
        Ok(Self { root })
    }

    fn capsules_dir(&self) -> PathBuf {
        self.root.join("capsules")
    }
    fn ann_dir(&self) -> PathBuf {
        self.root.join("annotations")
    }
    fn handle_dir(&self) -> PathBuf {
        self.root.join("handles")
    }

    /// Ingest a capsule: assign id if absent, compute canonical hash, and persist JSON.
    /// Non-blocking evaluation lives outside; this is purely storage + integrity.
    pub fn ingest_capsule(&self, mut cap: SimCapsule) -> Result<CapsHandle> {
        // Assign id if missing
        if cap.meta.capsule_id.is_none() {
            cap.meta.capsule_id = Some(uuidv7());
        }
        let id = cap.meta.capsule_id.clone().unwrap();

        // Compute canonical hash (over canonicalized JSON without the capsule_hash field)
        let mut v = serde_json::to_value(&cap).context("serialize capsule")?;
        if let Some(m) = v.get_mut("meta").and_then(|m| m.as_object_mut()) {
            m.remove("capsule_hash");
        }
        let hash = canonical_hash(&v);
        cap.meta.capsule_hash = Some(hash.clone());

        // Persist capsule JSON pretty-printed for auditability
        let path = self.capsules_dir().join(format!("{}.json", sanitize(&id)));
        write_atomic(&path, &serde_json::to_vec_pretty(&cap)?)?;

        // Optional dev auto-allow (scoped) — controlled via env vars.
        maybe_dev_auto_allow(self, &cap, &id)?;

        Ok(CapsHandle { id, hash })
    }

    /// Append an annotation entry for a capsule (JSONL per capsule id) and write latest.json.
    pub fn annotate(&self, id: &CapsId, annot: &CapsAnnot) -> Result<()> {
        let dir = self.ann_dir();
        let file = dir.join(format!("{}.jsonl", sanitize(id)));
        // append line
        let line = serde_json::to_vec(annot)?;
        append_line(&file, &line)?;

        // also write latest.json (overwrite)
        let latest = dir.join(format!("{}.latest.json", sanitize(id)));
        write_atomic(&latest, &serde_json::to_vec_pretty(annot)?)?;
        Ok(())
    }

    /// Read the latest annotation if available.
    pub fn latest_annotation(&self, id: &CapsId) -> Result<Option<CapsAnnot>> {
        let latest = self.ann_dir().join(format!("{}.latest.json", sanitize(id)));
        if !latest.exists() {
            return Ok(None);
        }
        let bytes = fs::read(&latest)?;
        let v: CapsAnnot = serde_json::from_slice(&bytes).context("parse latest annot")?;
        Ok(Some(v))
    }

    /// Hard gate for replay/use surfaces.
    /// Policy: if missing annotation → deny as pending; AllowWithPatch → caller applies patch.
    pub fn gate_replay(&self, id: &CapsId, _purpose: Purpose) -> std::result::Result<(), Denied> {
        match self.latest_annotation(id).map_err(|e| Denied {
            reason: format!("store error: {e}"),
            verdict: Verdict::Quarantine,
            risk: 1.0,
            labels: vec!["store_error".into()],
        })? {
            Some(ann) => {
                // DEV_ALLOW TTL enforcement: if enabled and expired, deny.
                if ann.labels.iter().any(|l| l == "DEV_ALLOW") {
                    if let Some(ttl_ms) = std::env::var("COGNIV_DEV_AUTO_ALLOW_TTL_MS")
                        .ok()
                        .and_then(|s| s.parse::<u64>().ok())
                    {
                        let now_ms = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64;
                        if now_ms.saturating_sub(ann.ts_ms) > ttl_ms {
                            return Err(Denied {
                                reason: "dev_allow_expired".into(),
                                verdict: Verdict::Quarantine,
                                risk: 1.0,
                                labels: ann.labels,
                            });
                        }
                    }
                }
                match ann.verdict {
                    Verdict::Allow | Verdict::AllowWithPatch => Ok(()),
                    Verdict::Quarantine => Err(Denied {
                        reason: "quarantined".into(),
                        verdict: Verdict::Quarantine,
                        risk: ann.risk,
                        labels: ann.labels,
                    }),
                }
            }
            None => Err(Denied {
                reason: "annotation_pending".into(),
                verdict: Verdict::Quarantine,
                risk: 1.0,
                labels: vec!["pending".into()],
            }),
        }
    }

    /// Optional mapping helpers (memory_id → capsule_id) for quick lookup by services.
    pub fn map_memory(&self, memory_id: &str, caps_id: &CapsId) -> Result<()> {
        let p = self
            .handle_dir()
            .join("memory")
            .join(format!("{}.txt", sanitize(memory_id)));
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent)?;
        }
        write_atomic(&p, caps_id.as_bytes())
    }

    /// Load capsule id for a given memory row, if mapped.
    pub fn capsule_for_memory(&self, memory_id: &str) -> Result<Option<CapsId>> {
        let p = self
            .handle_dir()
            .join("memory")
            .join(format!("{}.txt", sanitize(memory_id)));
        if !p.exists() {
            return Ok(None);
        }
        let bytes = fs::read(&p)?;
        let s = String::from_utf8_lossy(&bytes).trim().to_string();
        if s.is_empty() {
            Ok(None)
        } else {
            Ok(Some(s))
        }
    }

    pub fn load_capsule(&self, id: &CapsId) -> Result<Option<SimCapsule>> {
        let p = self.capsules_dir().join(format!("{}.json", sanitize(id)));
        if !p.exists() {
            return Ok(None);
        }
        let bytes = fs::read(&p)?;
        let v: SimCapsule = serde_json::from_slice(&bytes).context("parse capsule json")?;
        Ok(Some(v))
    }
}

// -------------- helpers --------------

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(".tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

fn append_line(path: &Path, line: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    f.write_all(line)?;
    f.write_all(b"\n")?;
    Ok(())
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

/// Deterministic JSON canonicalization (sort object keys recursively) then Blake3 hash hex.
fn canonical_hash(v: &Value) -> String {
    let cv = canonicalize(v);
    let bytes = serde_json::to_vec(&cv).expect("canonical json");
    let h = blake3::hash(&bytes);
    h.to_hex().to_string()
}

fn canonicalize(v: &Value) -> Value {
    match v {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let mut out = serde_json::Map::new();
            for k in keys {
                out.insert(k.clone(), canonicalize(&map[k]));
            }
            Value::Object(out)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(canonicalize).collect()),
        _ => v.clone(),
    }
}

// ---------------- dev auto-allow helper ----------------

fn maybe_dev_auto_allow(store: &ContractsStore, cap: &SimCapsule, id: &CapsId) -> Result<()> {
    // Enabled?
    let enabled = std::env::var("COGNIV_DEV_AUTO_ALLOW")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if !enabled {
        return Ok(());
    }

    // Namespace filter
    let prefix =
        std::env::var("COGNIV_DEV_AUTO_ALLOW_PREFIX").unwrap_or_else(|_| "dev/".to_string());
    let lobe_ok = cap
        .meta
        .lobe
        .as_deref()
        .map(|l| l.starts_with(&prefix))
        .unwrap_or(false);
    if !lobe_ok {
        return Ok(());
    }

    // Size cap (bytes of outputs serialized)
    let max_bytes: usize = std::env::var("COGNIV_DEV_AUTO_ALLOW_MAX_BYTES")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(64 * 1024);
    let out_bytes = serde_json::to_vec(&cap.outputs).unwrap_or_default();
    if out_bytes.len() > max_bytes {
        return Ok(());
    }

    // Annotate immediately as Allow (dev) with label DEV_ALLOW
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let annot = CapsAnnot {
        verdict: Verdict::Allow,
        risk: 0.0,
        labels: vec!["DEV_ALLOW".into()],
        policy_ver: "dev".into(),
        patch_id: None,
        ts_ms: now_ms,
    };
    let _ = store.annotate(id, &annot);
    eprintln!(
        "[contracts] DEV_ALLOW applied to capsule {id} (lobe prefix {prefix}), expires per TTL env"
    );
    Ok(())
}
