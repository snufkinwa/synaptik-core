use anyhow::{Context, Result};
use blake3;
use std::{
    borrow::Cow,
    fs,
    path::{Path, PathBuf},
};

/// === Embedded canon contracts ===
pub const NONVIOLENCE_TOML_NAME: &str = "nonviolence.toml";
pub const NONVIOLENCE_TOML: &str = include_str!("../assets/nonviolence.toml");

/// Return the embedded text for a known contract, if any.
pub fn default_contract_text(name: &str) -> Option<&'static str> {
    match name {
        NONVIOLENCE_TOML_NAME => Some(NONVIOLENCE_TOML),
        _ => None,
    }
}

/// Seed missing default contracts into a destination directory (idempotent).
/// Returns a list of files that were created.
pub fn write_default_contracts(dir: impl AsRef<Path>) -> Result<Vec<String>> {
    let dir = dir.as_ref();
    fs::create_dir_all(dir).with_context(|| format!("create_dir_all({:?})", dir))?;

    let mut created = Vec::new();

    for (name, text) in [(NONVIOLENCE_TOML_NAME, NONVIOLENCE_TOML)] {
        let path = dir.join(name);
        if !path.exists() {
            fs::write(&path, text).with_context(|| format!("write {:?}", path))?;
            created.push(name.to_string());
        }
    }

    Ok(created)
}

/// Verified reader with “locked” mode.
///
/// - If `path` exists:
///   - compute blake3(file) and compare to blake3(embedded) **if** we know an embedded copy.
///   - if hashes match → return file contents.
///   - if mismatch and `locked`:
///       * overwrite file with embedded
///       * return embedded
///   - if mismatch and **not** locked:
///       * return file (but you can log a warning upstream)
///
/// - If `path` missing and we know the embedded copy:
///   - if parent dir exists (or can be created), write the embedded to disk
///   - return embedded
///
/// - If we don’t have an embedded copy for `name`, just try to read the file best-effort.
pub fn read_verified_or_embedded(
    path: &Path,
    name: &str,
    locked: bool,
) -> Result<Cow<'static, str>> {
    let embedded_opt = default_contract_text(name);

    // Try reading local file if present.
    if path.exists() {
        let file_bytes = fs::read(path).with_context(|| format!("read {:?}", path))?;
        if let Some(embedded) = embedded_opt {
            let file_hash = blake3::hash(&file_bytes).to_hex().to_string();
            let embedded_hash = blake3::hash(embedded.as_bytes()).to_hex().to_string();
            if file_hash == embedded_hash {
                // Verified
                return Ok(Cow::Owned(String::from_utf8_lossy(&file_bytes).to_string()));
            }
            // Mismatch
            if locked {
                // Auto-heal: restore canonical embedded
                if let Some(dir) = path.parent() {
                    fs::create_dir_all(dir).ok();
                }
                fs::write(path, embedded)
                    .with_context(|| format!("restore embedded {:?}", path))?;
                return Ok(Cow::Borrowed(embedded));
            } else {
                // Allow local edits in unlocked mode
                return Ok(Cow::Owned(String::from_utf8_lossy(&file_bytes).to_string()));
            }
        } else {
            // Unknown name: no embedded baseline; return local as-is
            return Ok(Cow::Owned(String::from_utf8_lossy(&file_bytes).to_string()));
        }
    }

    // File missing: if we have embedded, write it; else return empty.
    if let Some(embedded) = embedded_opt {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).ok();
        }
        // Best-effort write; ignore failures (caller still gets embedded in memory)
        let _ = fs::write(path, embedded);
        Ok(Cow::Borrowed(embedded))
    } else {
        Ok(Cow::Owned(String::new()))
    }
}

/// Convenience: resolve `<root>/contracts/<name>`
pub fn contracts_path(root: &Path, name: &str) -> PathBuf {
    root.join("contracts").join(name)
}
