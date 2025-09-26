use anyhow::{anyhow, Result};
use serde_json::{Value, json};

use crate::commands::Commands;
use crate::memory::dag::MemoryState as DagMemoryState;
use crate::services::audit::record_action;
use crate::services::ethos::{Decision, decision_gate, precheck};

impl Commands {
    /// Recall an immutable snapshot by content hash.
    pub fn replay_recall_snapshot(&self, snapshot_id: &str) -> Result<DagMemoryState> {
        crate::memory::dag::recall_snapshot(snapshot_id)
    }

    /// Create or reset a named path diverging from the given snapshot.
    pub fn replay_diverge_from(&self, snapshot_id: &str, path_name: &str) -> Result<String> {
        crate::memory::dag::diverge_from(snapshot_id, path_name)
    }

    /// Append a new immutable snapshot to a named path and advance its head. Returns new hash.
    pub fn replay_extend_path(&self, path_name: &str, state: DagMemoryState) -> Result<String> {
        let new_id = self.memory.extend_path(path_name, state)?;
        record_action(
            "commands",
            "replay_extend_path",
            &json!({ "path_name": path_name, "new_hash": new_id }),
            "low",
        );
        Ok(new_id)
    }

    /// Fast-forward the target path to the source head.
    pub fn systems_consolidate(&self, src_path: &str, dst_path: &str) -> Result<String> {
        let src_head = self.dag_head(src_path)?.ok_or(anyhow!("no src head"))?;
        if let Some(dst_head) = self.dag_head(dst_path)? {
            if dst_head == src_head { return Ok(src_head); }
            if crate::memory::dag::is_ancestor(&dst_head, &src_head)? {
                self.update_path_head(dst_path, &src_head)?;
            } else {
                return Err(anyhow!("non-fast-forward: dst is not ancestor of src"));
            }
        } else {
            self.update_path_head(dst_path, &src_head)?;
        }
        Ok(src_head)
    }

    /// Create a bind snapshot with parents [feature_head, main_head] and move main to it.
    pub fn reconsolidate_paths(&self, main_path: &str, feature_path: &str, _note: &str) -> Result<String> {
        let main_head = self.dag_head(main_path)?.ok_or(anyhow!("no main head"))?;
        let feat_head = self.dag_head(feature_path)?.ok_or(anyhow!("no feature head"))?;
        if main_head == feat_head { return Ok(main_head); }
        if crate::memory::dag::is_ancestor(&main_head, &feat_head)? {
            self.update_path_head(main_path, &feat_head)?;
            return Ok(feat_head);
        }

        let lca = crate::memory::dag::bind_base(&main_head, &feat_head)?;
        let base_text = match lca.as_deref() { Some(h) => crate::memory::dag::recall_snapshot(h)?.content, None => String::new() };
        let left_text = crate::memory::dag::recall_snapshot(&feat_head)?.content;
        let right_text = crate::memory::dag::recall_snapshot(&main_head)?.content;

        let (bindd_text, had_conflicts) = crate::commands::bind::three_way_bind_lines(&base_text, &left_text, &right_text);

        let mut quarantined = false;
        let mut constraints: Vec<String> = Vec::new();
        let mut risk = String::new();
        let mut reason = String::new();
        if self.config().services.ethos_enabled {
            match precheck(&bindd_text, "replay_bind") {
                Ok(v) => {
                    let d = decision_gate(&v);
                    constraints = v.constraints.clone();
                    risk = v.risk.clone();
                    reason = v.reason.clone();
                    if matches!(d, Decision::Block) { quarantined = true; }
                }
                Err(_) => { quarantined = true; }
            }
        }

        let parents_capsules = || -> Vec<String> {
            let mut out = Vec::new();
            for h in [&feat_head, &main_head] {
                if let Ok(m) = crate::memory::dag::snapshot_meta(h) {
                    if let Some(cid) = m.get("capsule_id").and_then(|x| x.as_str()) { out.push(cid.to_string()); }
                }
            }
            out
        }();
        let enrich = json!({
            "op": "bind",
            "actor": "core",
            "parents_cids": [feat_head, main_head],
            "parents_capsules": parents_capsules,
            "lca": lca,
            "bind_conflicts": had_conflicts,
            "quarantined": quarantined,
            "policy_constraints": constraints,
            "policy_risk": risk,
            "policy_reason": reason,
            "note": _note,
        });

        let mut meta_obj = serde_json::Map::new();
        if let Value::Object(m) = enrich { meta_obj = m; }
         let new_file = crate::memory::dag::save_node(
             &blake3::hash(bindd_text.as_bytes()).to_hex().to_string(),
             &bindd_text,
             &Value::Object(meta_obj),
             &[feat_head.clone(), main_head.clone()],
         )?;
        let new_hash = {
            let v = crate::memory::dag::load_node(&new_file)?;
            v.get("hash")
                .and_then(|x| x.as_str())
                .ok_or_else(|| anyhow!("saved bind node is missing its hash"))?
                .to_string()
        };
        self.update_path_head(main_path, &new_hash)?;
        record_action(
            "commands",
            "bind_created",
            &json!({ "main_path": main_path, "feature_path": feature_path, "hash": new_hash, "conflicts": had_conflicts, "quarantined": quarantined }),
            if quarantined { "high" } else { "low" },
        );
        Ok(new_hash)
    }

    /// Idempotent, normalized: create a branch at a resolved base (cid|path|lobe).
    pub fn branch(&self, path: &str, base: Option<&str>, lobe: Option<&str>) -> Result<String> {
        let path_norm = self.normalize_path_name(path);
        if crate::memory::dag::path_exists(&path_norm)? {
            if let Some(b) = crate::memory::dag::path_base_snapshot(&path_norm)? { return Ok(b); }
        }
        let resolved_base = if let Some(b) = base {
            let b_norm = self.normalize_path_name(b);
            if crate::memory::dag::path_exists(&b_norm)? { self.dag_head(&b_norm)? } else { Some(b.to_string()) }
        } else if let Some(l) = lobe { self.replay_base_from_lobe(l)? }
          else if let Some(h) = self.dag_head("main")? { Some(h) }
          else { self.replay_base_from_lobe("chat")? }
        .ok_or(anyhow!("no base available to branch from"))?;

        let _ = self.replay_diverge_from(&resolved_base, &path_norm)?;
        record_action("commands", "branch_created", &json!({ "path": path_norm, "base": resolved_base }), "low");
        Ok(resolved_base)
    }

    /// Append content to a named path with provenance and ethos gating.
    pub fn append(&self, path: &str, content: &str, meta: Option<Value>) -> Result<String> {
        let path_norm = self.normalize_path_name(path);
        if !crate::memory::dag::path_exists(&path_norm)? { return Err(anyhow!(format!("path '{}' not found; call branch() first", path_norm))); }

        let governed_text = if self.config().services.ethos_enabled {
            match self.govern_text("replay_append", content) {
                Ok(Some(s)) => s,
                Ok(None) => return Err(anyhow!("blocked by runtime")),
                Err(e) => return Err(anyhow!("runtime error: {}", e)),
            }
        } else { content.to_string() };

        let parent = self.dag_head(&path_norm)?;
        let base = crate::memory::dag::path_base_snapshot(&path_norm)?;
        let enrich = json!({
            "op": "append",
            "ts": chrono::Utc::now().to_rfc3339(),
            "actor": "core",
            "path": path_norm,
            "parents": parent.clone().into_iter().collect::<Vec<_>>(),
            "base": base,
            "content_hash": blake3::hash(governed_text.as_bytes()).to_hex().to_string(),
        });
        let bindd_meta = match meta.unwrap_or_else(|| json!({})) {
            Value::Object(mut m) => { if let Value::Object(e) = enrich { m.extend(e); } Value::Object(m) }
            _ => enrich,
        };
        let state = DagMemoryState { content: governed_text, meta: bindd_meta };
        let id = self.replay_extend_path(&path_norm, state)?;
        Ok(id)
    }
}
