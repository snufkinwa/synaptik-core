use std::path::PathBuf;

use synaptik_core::commands::Commands;
use synaptik_core::memory::dag::is_ancestor;

fn temp_root(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "synaptik_core_test_{}_{}",
        name,
        std::process::id()
    ));
    p
}

#[test]
fn branch_append_and_consolidate_fast_forward() {
    // Isolate DAG/DB paths under a temporary root
    let root = temp_root("replay");
    // Clean if exists from previous runs
    let _ = std::fs::remove_dir_all(&root);
    unsafe {
        std::env::set_var("COGNIV_ROOT", &root);
    }

    let cmd = Commands::new("ignored", None).expect("commands new");

    // Create branch from lobe seed
    let base = cmd
        .branch("feature_path", None, Some("chat"))
        .expect("branch created");

    // Append two snapshots
    let s1 = cmd
        .append("feature_path", "first content", None)
        .expect("append 1");
    let s2 = cmd
        .append("feature_path", "second content", None)
        .expect("append 2");
    assert_ne!(s1, s2);

    // Head should be latest append
    let head = cmd
        .dag_head("feature_path")
        .expect("head check")
        .expect("some head");
    assert_eq!(head, s2);

    // Ancestor chain should include base -> s2
    assert!(is_ancestor(&base, &s2).expect("ancestor check"));

    // Consolidate to main via fast-forward
    let main_head = cmd
        .consolidate("feature_path", "main")
        .expect("consolidate ff");
    assert_eq!(main_head, s2);
    let main_head2 = cmd
        .dag_head("main")
        .expect("get main head")
        .expect("some main head");
    assert_eq!(main_head2, s2);

    // Cleanup temp root
    let _ = std::fs::remove_dir_all(&root);
}
