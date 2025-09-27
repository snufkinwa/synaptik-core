use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Ensure that a candidate absolute path resolves (or its parent resolves) to a
/// location contained within the canonicalized `root_abs`. Returns the
/// canonicalized path that was checked.
///
/// For creation paths that don't exist yet, this canonicalizes the parent and
/// rejoins the leaf to prevent symlink escapes.
pub fn assert_within_root_abs(root_abs: &Path, candidate_abs: &Path) -> Result<PathBuf> {
    let root = root_abs
        .canonicalize()
        .with_context(|| format!("canonicalize root {:?}", root_abs))?;

    // Try to canonicalize the full path; if it doesn't exist, canonicalize parent
    // and then re-attach the filename.
    let resolved = match candidate_abs.canonicalize() {
        Ok(c) => c,
        Err(_) => {
            let parent = candidate_abs
                .parent()
                .ok_or_else(|| anyhow::anyhow!("invalid path: no parent"))?;
            let leaf = candidate_abs
                .file_name()
                .ok_or_else(|| anyhow::anyhow!("invalid path: no file name"))?;
            let canon_parent = parent
                .canonicalize()
                .with_context(|| format!("canonicalize parent {:?}", parent))?;
            canon_parent.join(leaf)
        }
    };

    if !resolved.starts_with(&root) {
        anyhow::bail!(
            "path escapes root: path={:?} root={:?}",
            candidate_abs, root
        );
    }
    Ok(resolved)
}

/// Join a relative path onto a canonicalized root and return the absolute path.
/// Rejects absolute inputs.
pub fn resolve_rel_within_root(root_abs: &Path, rel: &Path) -> Result<PathBuf> {
    if rel.is_absolute() {
        anyhow::bail!("absolute paths are not allowed");
    }
    let root = root_abs
        .canonicalize()
        .with_context(|| format!("canonicalize root {:?}", root_abs))?;
    Ok(root.join(rel))
}

