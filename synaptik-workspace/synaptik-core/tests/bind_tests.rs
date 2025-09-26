use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;
use synaptik_core::commands::Commands;
use synaptik_core::commands::ensure_initialized_once;

#[test]
fn bind_creates_two_parent_node_and_updates_head() -> anyhow::Result<()> {
    // Use unique path names to avoid cross-test interference
    let ns = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let main_path = format!("main_bind_{}", ns);
    let feat_path = format!("feat_bind_{}", ns);

    let cmds = Commands::new("ignored", None)?;

    // Create branches from the same lobe base (chat)
    let base = cmds.branch(&main_path, None, Some("chat"))?;
    let _ = cmds.branch(&feat_path, Some(&base), None)?;

    // Diverge both paths
    let _ = cmds.append(&main_path, "main content A", None)?;
    let _ = cmds.append(&feat_path, "feature content B", None)?;

    // Capture heads before bind
    let main_head = cmds.dag_head(&main_path)?.expect("main head");
    let feat_head = cmds.dag_head(&feat_path)?.expect("feat head");

    // Non-FF bind: create two-parent node and move main
    let binding = cmds.reconsolidate_paths(&main_path, &feat_path, "test bind")?;
    let new_head = cmds.dag_head(&main_path)?.expect("new main head");
    assert_eq!(binding, new_head);

    // Parents are recorded in node JSON as ordered array [feat_head, main_head]
    let report = ensure_initialized_once().expect("init");
    let idx_path = report
        .root
        .join("refs")
        .join("hashes")
        .join(format!("{}.json", binding));
    let idx_bytes = std::fs::read(&idx_path)?;
    let idx_v: Value = serde_json::from_slice(&idx_bytes)?;
    let node_name = idx_v.get("node").and_then(|x| x.as_str()).unwrap_or("");
    let node_path = report.root.join("dag").join("nodes").join(node_name);
    let node_v: Value = serde_json::from_slice(&std::fs::read(&node_path)?)?;
    let parents: Vec<String> = node_v
        .get("parents")
        .and_then(|x| x.as_array())
        .unwrap()
        .iter()
        .filter_map(|s| s.as_str().map(|t| t.to_string()))
        .collect();
    assert_eq!(parents.len(), 2);
    assert_eq!(parents[0], feat_head);
    assert_eq!(parents[1], main_head);

    // LCA should be the original base
    let lca = cmds.replay_bind_base(&main_head, &feat_head)?;
    assert_eq!(lca.as_deref(), Some(base.as_str()));

    Ok(())
}

