use std::time::{SystemTime, UNIX_EPOCH};

use contracts::api::{CapsAnnot, Purpose, Verdict};
use contracts::capsule::{CapsuleMeta, CapsuleSource, SimCapsule};
use contracts::store::ContractsStore;

fn tmp_dir(name: &str) -> std::path::PathBuf {
    let ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("simcaps_{ns}_{name}"))
}

#[test]
fn simcapsule_ingest_annotate_and_gate() {
    let root = tmp_dir("store");
    let store = ContractsStore::new(&root).expect("store");

    let now_ms = std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let cap = SimCapsule {
        inputs: serde_json::json!({}),
        context: serde_json::json!({"lobe": "chat"}),
        actions: serde_json::json!(["ingest_text"]),
        outputs: serde_json::json!({"text": "hello"}),
        trace: serde_json::json!({}),
        artifacts: vec![],
        meta: CapsuleMeta {
            capsule_id: None,
            agent_id: Some("core".into()),
            lobe: Some("chat".into()),
            t_start_ms: now_ms,
            t_end_ms: now_ms,
            source: CapsuleSource::Real,
            schema_ver: "1.0".into(),
            capsule_hash: None,
            issuer_signature: None,
            parent_id: None,
        },
    };

    let handle = store.ingest_capsule(cap).expect("ingest");
    assert!(!handle.id.is_empty());
    assert!(!handle.hash.is_empty());

    // Gate should deny pending (no annotation yet)
    let deny = store
        .gate_replay(&handle.id, Purpose::Replay)
        .err()
        .expect("deny pending");
    assert_eq!(deny.reason, "annotation_pending");

    // Annotate allow and gate should pass
    let ann = CapsAnnot {
        verdict: Verdict::Allow,
        risk: 0.0,
        labels: vec!["ok".into()],
        policy_ver: "test".into(),
        patch_id: None,
        ts_ms: now_ms,
    };
    store.annotate(&handle.id, &ann).expect("annotate");
    store
        .gate_replay(&handle.id, Purpose::Replay)
        .expect("gate allow");

    // Map memory id to capsule id and resolve back
    let mem_id = "chat_abc".to_string();
    store.map_memory(&mem_id, &handle.id).expect("map");
    let back = store.capsule_for_memory(&mem_id).expect("map lookup");
    assert_eq!(back.as_deref(), Some(handle.id.as_str()));
}
