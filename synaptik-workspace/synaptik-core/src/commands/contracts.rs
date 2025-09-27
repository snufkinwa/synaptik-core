use anyhow::{Result, anyhow};
use std::fs;
use std::path::{Path, PathBuf};

use crate::commands::bind::three_way_bind_lines;
use crate::commands::init::ensure_initialized_once;
use crate::services::audit::record_action;
use contracts::assets::default_contract_text;

use super::Commands;

/// Contracts admin helpers: merge (bind) and pull (reinstatement).
impl Commands {
    /// Reinstatement (pull): restore the default contract file from the embedded canon.
    ///
    /// - Overwrites the configured default contract under `.cogniv/contracts/` with the embedded copy.
    /// - Returns the absolute path that was restored.
    /// - Safe to call repeatedly (idempotent).
    pub fn contracts_reinstatement(&self) -> Result<PathBuf> {
        let cfg = ensure_initialized_once()?.config.clone();
        let dir = cfg.contracts.path.clone();
        let name = cfg.contracts.default_contract.clone();
        let target = dir.join(&name);

        let embedded = match default_contract_text(&name) {
            Some(t) => t,
            None => {
                return Err(anyhow!(
                    "no embedded canon for contract {:?}; cannot reinstate",
                    name
                ));
            }
        };
        if let Some(parent) = target.parent() {
            let _ = fs::create_dir_all(parent);
        }
        fs::write(&target, embedded)?;

        record_action(
            "commands",
            "contracts_reinstated",
            &serde_json::json!({ "path": target.to_string_lossy() }),
            "low",
        );
        Ok(target)
    }

    /// Bind (merge): three-way merge an incoming contract text against the current local file,
    /// using the embedded canon as the base.
    ///
    /// Returns (binding_text, had_conflicts). If `apply_path` is `Some`, writes the binding
    /// result to that path (or to the configured default contract if `apply_path` is None
    /// and `apply` is true).
    pub fn contracts_bind_with_text(
        &self,
        incoming_text: &str,
        apply: bool,
        apply_path: Option<&Path>,
    ) -> Result<(String, bool)> {
        let cfg = ensure_initialized_once()?.config.clone();
        let dir = cfg.contracts.path.clone();
        let name = cfg.contracts.default_contract.clone();
        let local_path = dir.join(&name);

        // Base: embedded canon (may be empty if unknown)
        let base_text = default_contract_text(&name).unwrap_or("");

        // Right: local file contents (best-effort)
        let right_text = match fs::read_to_string(&local_path) {
            Ok(s) => s,
            Err(_) => String::new(),
        };

        // Left: incoming text provided by caller
        let left_text = incoming_text;

        let (binding, had_conflicts) = three_way_bind_lines(base_text, left_text, &right_text);

        // Audit a preview event (without content)
        record_action(
            "commands",
            "contracts_bind_preview",
            &serde_json::json!({
                "default_contract": name,
                "had_conflicts": had_conflicts,
                "apply": apply,
            }),
            "low",
        );

        if apply {
            let target = apply_path
                .map(|p| p.to_path_buf())
                .unwrap_or(local_path.clone());
            if let Some(parent) = target.parent() {
                let _ = fs::create_dir_all(parent);
            }
            fs::write(&target, binding.as_bytes())?;
            record_action(
                "commands",
                "contracts_bind_applied",
                &serde_json::json!({
                    "path": target.to_string_lossy(),
                    "had_conflicts": had_conflicts
                }),
                "medium",
            );
        }

        Ok((binding, had_conflicts))
    }

    /// Bind (merge) from a file containing incoming contract text. Convenience wrapper over `contracts_bind_with_text`.
    pub fn contracts_bind_from_path(
        &self,
        incoming_path: &Path,
        apply: bool,
        apply_path: Option<&Path>,
    ) -> Result<(String, bool)> {
        let text = fs::read_to_string(incoming_path)
            .map_err(|e| anyhow!("read {:?}: {}", incoming_path, e))?;
        self.contracts_bind_with_text(&text, apply, apply_path)
    }

    /// Alias: reinstatement (pull) — convenience entrypoint with canonical naming.
    pub fn reinstatement(&self) -> Result<PathBuf> {
        self.contracts_reinstatement()
    }
}
