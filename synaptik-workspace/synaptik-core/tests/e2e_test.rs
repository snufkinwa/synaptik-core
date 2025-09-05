// tests/e2e.rs
// End-to-end tests for Synaptik MVP: Memory + Librarian + Archivist + Commands
//
// Run with: cargo test -- --nocapture

use blake3;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use std::{thread, time::Duration};

use rusqlite::Connection;

use synaptik_core::commands::Commands;
use synaptik_core::services::archivist::Archivist;
use synaptik_core::services::librarian::Librarian;
use synaptik_core::services::memory::Memory;

use synaptik_core::commands::ensure_initialized_once;

static COUNTER: AtomicU64 = AtomicU64::new(0);

// Serialize contract-mutation tests to avoid cross-test races on shared .cogniv/contracts
fn contract_test_guard() -> std::sync::MutexGuard<'static, ()> {
    use std::sync::{Mutex, Once};
    static mut PTR: *const Mutex<()> = std::ptr::null();
    static INIT: Once = Once::new();
    unsafe {
        INIT.call_once(|| {
            let b = Box::new(Mutex::new(()));
            PTR = Box::into_raw(b);
        });
        (&*PTR).lock().unwrap()
    }
}

fn tmp_path(name: &str) -> PathBuf {
    let ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let pid = std::process::id();
    let c = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("synaptik_e2e_{pid}_{ns}_{c}_{name}"))
}

fn open_sqlite<P: AsRef<Path>>(p: P) -> Connection {
    if let Some(parent) = p.as_ref().parent() {
        std::fs::create_dir_all(parent).ok();
    }
    Connection::open(p).expect("open sqlite")
}

#[test]
fn commands_remember_reflect_stats() {
    // Commands resolves canonical paths internally; the db_path arg is ignored.
    let cmds = Commands::new("ignored", None).expect("commands new");

    // Act: remember (always summarizes for long text; here it's short, ok)
    let content = "User prefers concise explanations. They like short answers. This is a test.";
    let memory_id = cmds.remember("chat", None, content).expect("remember");

    // Act: reflect over recent summaries for this lobe
    let note = cmds.reflect("chat", 20).expect("reflect");
    assert!(note.len() <= 256, "reflection should be short");

    // Act: stats
    let stats = cmds.stats(None).expect("stats");
    assert!(stats.total >= 1, "at least one row should exist");
    assert!(stats.last_updated.is_some());

    // Assert against the canonical DB under .cogniv (via init)
    let report = ensure_initialized_once().expect("init");
    let db_path = report.root.join("cache").join("memory.db");
    let conn = open_sqlite(&db_path);

    let got_reflection: Option<String> = conn
        .query_row(
            "SELECT reflection FROM memories WHERE memory_id=?1",
            [memory_id.as_str()],
            |r| r.get(0),
        )
        .ok();
    assert!(got_reflection.is_some());
}

/// End-to-end test of recent() + recall()
#[test]
fn commands_recent_and_recall_roundtrip() {
    let cmds = Commands::new("ignored", None).expect("commands new");

    // write two memories to 'chat'
    let m1 = cmds
        .remember("chat", Some("k1"), "hello e2e one")
        .expect("remember 1");
    std::thread::sleep(std::time::Duration::from_millis(30));
    let m2 = cmds
        .remember("chat", Some("k2"), "hello e2e two")
        .expect("remember 2");

    // recent across the whole suite; ask for a few
    let ids = cmds.recent("chat", 10).expect("recent");
    let p2 = ids.iter().position(|x| x == &m2).expect("m2 present");
    let p1 = ids.iter().position(|x| x == &m1).expect("m1 present");
    assert!(p2 < p1, "newest (m2) should be before oldest (m1)");

    // recall → raw content
    let got2 = cmds.recall(&m2).expect("recall 2").unwrap();
    assert_eq!(got2, "hello e2e two");
    let got1 = cmds.recall(&m1).expect("recall 1").unwrap();
    assert_eq!(got1, "hello e2e one");
}

/// Prove precheck blocks harmful content
#[test]
fn commands_precheck_text_reports_decision() {
    let cmds = Commands::new("ignored", None).expect("commands new");

    // This should at least produce a structured decision under whatever contract is loaded
    let rep = cmds
        .precheck_text("I want to kill her", "chat_message")
        .expect("precheck");

    //assert!(["allow", "allow_with_constraints", "block"].contains(&rep.decision.as_str()));
    assert_eq!(rep.decision, "block");
    // …but keep in mind that will fail if the contract doesn’t have that rule.
}

#[test]
fn contract_lock_prevents_tampering() {
    let _g = contract_test_guard();
    let cmds = Commands::new("ignored", None).expect("commands new");
    let report = ensure_initialized_once().expect("init");
    let path = report.root.join("contracts").join("nonviolence.toml");

    let tampered = r#"name = "Tampered"
version = "0.0.1"
description = "tampered allow"

[[rules]]
action = "say"
effect = "allow"
matches_any = ["kill"]
severity = "none"
violation = "none"
"#;

    std::fs::write(&path, tampered).expect("write tampered");
    let rep = cmds
        .precheck_text("I want to kill her", "chat_message")
        .expect("precheck");
    assert_eq!(rep.decision, "block");

    cmds.unlock_contracts();
    std::fs::write(&path, tampered).expect("write tampered");
    let rep2 = cmds
        .precheck_text("I want to kill her", "chat_message")
        .expect("precheck");
    assert_eq!(rep2.decision, "allow");
    cmds.lock_contracts();
}

#[test]
fn librarian_promote_to_archive_and_restore() {
    // Setup: file-only Archivist (no DB), Memory single writer, Librarian orchestrator
    let root = tmp_path("archive_root");
    let db_path = tmp_path("mem.sqlite3");

    let archivist = Archivist::open(root.join("archive")).expect("archivist open");
    let mem = Memory::open(db_path.to_str().unwrap()).expect("mem open");
    let lib = Librarian::new(Some(archivist));

    // Ingest
    let content = "This is a large doc we want to cold-store.";
    let id = lib
        .ingest_text(&mem, "notes", None, content)
        .expect("ingest");

    // Promote to cold storage → Memory records archived_cid
    let cid = lib
        .promote_to_archive(&mem, &id)
        .expect("promote")
        .expect("cid");

    assert!(!cid.is_empty());

    // Fetch (cold path): Archivist.get → re-cache via Memory
    let fetched = lib.fetch(&mem, &id).expect("fetch").expect("some");
    assert_eq!(String::from_utf8_lossy(&fetched), content);
}

#[test]
fn memory_open_and_basic_io() {
    // Basic smoke test for Memory alone (single writer)
    let db_path = tmp_path("mem.sqlite3");
    let mem = Memory::open(db_path.to_str().unwrap()).expect("mem open");

    let id = "chat_abcd";
    mem.remember(id, "chat", "001", b"hello world")
        .expect("remember");
    let roundtrip = mem.recall(id).expect("recall").expect("some");
    assert_eq!(roundtrip, b"hello world");

    // with summary
    mem.remember_with_summary(id, "chat", "001", b"hello world", "short summary", None)
        .expect("remember_with_summary");

    // ensure summary is present
    let conn = open_sqlite(&db_path);
    let sum: Option<String> = conn
        .query_row(
            "SELECT summary FROM memories WHERE memory_id=?1",
            [id],
            |r| r.get(0),
        )
        .ok();
    assert_eq!(sum.as_deref(), Some("short summary"));
}

#[test]
fn memory_promote_all_hot_in_lobe_linear_chain() {
    // Arrange
    let db_path = tmp_path("mem.sqlite3");
    let mem = Memory::open(db_path.to_str().unwrap()).expect("mem open");

    // Write three hot rows in the same lobe. Space them slightly to stabilize created_at order.
    let id1 = "chat_001";
    let id2 = "chat_002";
    let id3 = "chat_003";

    let c1 = b"first payload";
    let c2 = b"second payload";
    let c3 = b"third payload";

    mem.remember(id1, "chat", "k1", c1).expect("remember 1");
    thread::sleep(Duration::from_millis(5));
    mem.remember(id2, "chat", "k2", c2).expect("remember 2");
    thread::sleep(Duration::from_millis(5));
    mem.remember(id3, "chat", "k3", c3).expect("remember 3");

    // Act: promote all hot rows in lobe (oldest → newest)
    let promoted = mem.promote_all_hot_in_lobe("chat").expect("promote_all");
    assert_eq!(promoted.len(), 3, "should promote all three rows");

    // Expect promotion order follows created_at ASC (id1, id2, id3)
    assert_eq!(promoted[0].0, id1);
    assert_eq!(promoted[1].0, id2);
    assert_eq!(promoted[2].0, id3);

    // Assert: archived_cid is blake3(content)
    let exp1 = blake3::hash(c1).to_hex().to_string();
    let exp2 = blake3::hash(c2).to_hex().to_string();
    let exp3 = blake3::hash(c3).to_hex().to_string();

    assert_eq!(promoted[0].1, exp1);
    assert_eq!(promoted[1].1, exp2);
    assert_eq!(promoted[2].1, exp3);

    // Assert: DB reflects archived_cid and archived_at is set
    let conn = open_sqlite(&db_path);
    for (id, exp_cid) in &promoted {
        let (cid, at): (Option<String>, Option<String>) = conn
            .query_row(
                "SELECT archived_cid, archived_at FROM memories WHERE memory_id=?1",
                [id.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("row exists");
        assert_eq!(cid.as_deref(), Some(exp_cid.as_str()));
        assert!(at.is_some(), "archived_at should be set for {}", id);
    }

    // Re-promoting should do nothing (already archived)
    let again = mem
        .promote_all_hot_in_lobe("chat")
        .expect("promote_all again");
    assert!(again.is_empty(), "no hot rows left to promote");
}

#[test]
fn memory_promote_latest_hot_in_lobe_single() {
    // Arrange
    let db_path = tmp_path("mem2.sqlite3");
    let mem = Memory::open(db_path.to_str().unwrap()).expect("mem open");

    // One archived row and one still hot
    mem.remember("chat_a1", "chat", "a1", b"old")
        .expect("remember a1");
    mem.promote_all_hot_in_lobe("chat").expect("promote a1"); // archive the first

    // new hot row
    mem.remember("chat_a2", "chat", "a2", b"new")
        .expect("remember a2");

    // Act: promote only the most recent hot row
    let one = mem
        .promote_latest_hot_in_lobe("chat")
        .expect("promote latest");
    assert!(one.is_some(), "should promote one row");
    let (id, cid) = one.unwrap();

    assert_eq!(id, "chat_a2");
    assert_eq!(cid, blake3::hash(b"new").to_hex().to_string());

    // Next call should find nothing hot
    let none = mem
        .promote_latest_hot_in_lobe("chat")
        .expect("promote latest none");
    assert!(none.is_none());
}

#[test]
fn contracts_enforced_on_disk_when_locked() {
    let _g = contract_test_guard();
    let cmds = Commands::new("ignored", None).expect("commands new");
    let report = ensure_initialized_once().expect("init");
    let path = report.root.join("contracts").join("nonviolence.toml");

    // Make sure we're in locked mode first
    cmds.lock_contracts();

    // Tamper the contract on disk
    let tampered = r#"name = "Tampered"
version = "0.0.1"
description = "tampered allow"

[[rules]]
action = "say"
effect = "allow"
matches_any = ["kill"]
severity = "none"
violation = "none"
"#;
    std::fs::write(&path, tampered).expect("write tampered");
    // Evaluate: should block and also restore canonical contract on disk
    let rep = cmds
        .precheck_text("I want to kill her", "chat_message")
        .expect("precheck");
    assert_eq!(rep.decision, "block");

    // Wait briefly for any concurrent writes to settle and for enforcement to persist.
    let mut restored = false;
    for _ in 0..20 {
        let after = std::fs::read_to_string(&path).expect("read after");
        if !after.contains("Tampered") {
            restored = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(restored, "locked evaluation should restore canonical contracts");

    // Now unlock, tamper again, and confirm the tamper persists and is honored
    cmds.unlock_contracts();
    std::fs::write(&path, tampered).expect("re-tamper");
    let rep2 = cmds
        .precheck_text("I want to kill her", "chat_message")
        .expect("precheck2");
    assert_eq!(rep2.decision, "allow");
    let after_unlock = std::fs::read_to_string(&path).expect("read after unlock");
    assert!(
        after_unlock.contains("Tampered"),
        "unlocked evaluation should not auto-restore contracts"
    );

    // Re-lock for other tests safety
    cmds.lock_contracts();
}

#[test]
fn commands_total_recall_degrades_to_dag_after_cache_miss() {
    let cmds = Commands::new("ignored", None).expect("commands new");

    // Ingest one memory in lobe "demo"
    let content = "demo payload for dag recall";
    let id = cmds
        .remember("demo", Some("k1"), content)
        .expect("remember");

    // Promote latest hot in lobe → writes DAG node and sets archived_cid
    let _ = cmds.promote_latest_hot("demo").expect("promote latest hot");

    // Remove the hot cache row to simulate cache miss
    let report = ensure_initialized_once().expect("init");
    let db_path = report.root.join("cache").join("memory.db");
    let conn = open_sqlite(&db_path);
    conn.execute("DELETE FROM memories WHERE memory_id=?1", [id.as_str()])
        .expect("delete row");

    // total recall: fall back to DAG and report source="dag"
    let res = cmds.recall_with_source(&id, None).expect("total_recall");
    let (got, source) = res.expect("some");
    assert_eq!(got, content);
    assert_eq!(source, "dag");

    // Archive-only still returns None here because we deleted the DB row above,
    // so there is no archived_cid to look up. Archive bytes may exist on disk,
    // but recall_archive requires the CID pointer from SQLite.
    let arch = cmds.recall_with_source(&id, Some("archive")).expect("recall_archive");
    let arch = arch.map(|(s, _)| s);
    assert!(arch.is_none());
}

#[test]
fn commands_total_recall_many_batch_uses_dag() {
    // Ensure contracts are in a stable, locked state to avoid cross-test races.
    let _g = contract_test_guard();
    let cmds = Commands::new("ignored", None).expect("commands new");
    cmds.lock_contracts();

    // Two memories in same lobe
    let id1 = cmds
        .remember("batch", Some("a"), "alpha content")
        .expect("remember a");
    let id2 = cmds
        .remember("batch", Some("b"), "beta content")
        .expect("remember b");

    // Promote all hot rows in this lobe via Memory API using the same DB
    let report = ensure_initialized_once().expect("init");
    let db_path = report.root.join("cache").join("memory.db");
    let mem = Memory::open(db_path.to_str().unwrap()).expect("mem open");
    mem.promote_all_hot_in_lobe("batch").expect("promote all hot");

    // Delete from hot cache to force fallback
    let conn = open_sqlite(&db_path);
    conn.execute("DELETE FROM memories WHERE memory_id=?1", [id1.as_str()])
        .expect("delete id1");
    conn.execute("DELETE FROM memories WHERE memory_id=?1", [id2.as_str()])
        .expect("delete id2");

    // Bulk total recall
    let out = cmds
        .total_recall_many(&vec![id1.clone(), id2.clone()], None)
        .expect("total_recall_many");

    // Expect 2 results, likely sourced from DAG (archive could also serve, both are acceptable)
    assert_eq!(out.len(), 2);
    for (rid, content, source) in out {
        assert!(source == "dag" || source == "archive", "unexpected source: {}", source);
        if rid == id1 {
            assert_eq!(content, "alpha content");
        } else if rid == id2 {
            assert_eq!(content, "beta content");
        } else {
            panic!("unexpected id {rid}");
        }
    }
}


/// Ethos risk ranking should reflect highest violated rule severity (High for personal threats)
#[test]
fn ethos_risk_ranking_personal_threat_is_high() {
    let cmds = Commands::new("ignored", None).expect("commands new");
    let _g = contract_test_guard();
    cmds.lock_contracts();
    let rep = cmds
        .precheck_text("I want to kill her", "chat_message")
        .expect("precheck");
    assert_eq!(rep.decision, "block");
    assert_eq!(rep.risk, "High");
    let have: std::collections::HashSet<_> = rep.constraints.iter().cloned().collect();
    for c in [
        "refuse_personal_harm_content",
        "avoid_violent_language",
        "reframe_nonviolent",
        "offer_deescalation",
        "do_not_repeat_harmful_phrases",
    ] {
        assert!(have.contains(c), "missing expected constraint: {}", c);
    }
}

/// Medium severity language should block with Medium risk under current policy
#[test]
fn ethos_risk_ranking_generic_harm_is_medium() {
    let cmds = Commands::new("ignored", None).expect("commands new");
    let _g = contract_test_guard();
    cmds.lock_contracts();
    let rep = cmds
        .precheck_text("I want to destroy all the things.", "chat_message")
        .expect("precheck");
    assert_eq!(rep.decision, "block");
    assert_eq!(rep.risk, "Medium");
    let have: std::collections::HashSet<_> = rep.constraints.iter().cloned().collect();
    for c in [
        "soften_language",
        "avoid_violent_language",
        "reframe_constructive",
    ] {
        assert!(have.contains(c), "missing expected constraint: {}", c);
    }
}

/// Memory precheck should allow benign technical idioms (per updated contract)
#[test]
fn commands_remember_allows_tech_idioms() {
    let _g = contract_test_guard();
    let cmds = Commands::new("ignored", None).expect("commands new");
    // Ensure embedded contract is active
    cmds.lock_contracts();

    // Phrase contains "kill bugs" but in a technical/benign sense (allowlisted)
    let content = "User wants an IDE that can automatically help kill bugs (hyperbolic).";

    // Precheck explicitly for memory_storage should not block
    let rep = cmds
        .precheck_text(content, "memory_storage")
        .expect("precheck memory_storage");
    assert!(
        rep.decision == "allow" || rep.decision == "allow_with_constraints",
        "unexpected decision: {} (risk={})",
        rep.decision,
        rep.risk
    );

    // And remember() should succeed (no ethics block)
    let id = cmds
        .remember("solutions", Some("ide_bug_killing"), content)
        .expect("remember with tech idiom allowed");
    assert!(!id.is_empty());
}

/// Recall parity: hot vs. archive vs. dag should yield identical content for the same id
#[test]
fn commands_recall_parity_across_tiers() {
    // Write a fresh row directly via Memory to avoid any precheck/policy interference.
    let report = ensure_initialized_once().expect("init");
    let db_path = report.root.join("cache").join("memory.db");
    let mem = Memory::open(db_path.to_str().unwrap()).expect("mem open");

    let lobe = "parity";
    let id = "parity_profile_1";
    let content = b"User Name: Alex\nRole: Engineer\nPrefers: concise answers.";
    mem.remember(id, lobe, "profile_1", content).expect("mem remember");

    // Promote this specific id to DAG (marks archived_cid) and write filesystem archive
    mem.promote_to_dag(id).expect("promote_to_dag");
    // Ensure archive object exists
    let arch = Archivist::open(report.root.join("archive")).expect("arch open");
    let bytes = mem.recall(id).expect("recall bytes").expect("some bytes");
    let _ = arch.archive(id, &bytes).expect("archive bytes");

    let cmds = Commands::new("ignored", None).expect("commands new");

    // Recall with explicit sources
    let hot = cmds.recall_with_source(id, Some("hot")).expect("recall_hot").map(|(s, _)| s).expect("hot some");
    let arc = cmds.recall_with_source(id, Some("archive")).expect("recall_archive").map(|(s, _)| s).expect("arc some");
    let dag = cmds.recall_with_source(id, Some("dag")).expect("recall_dag").map(|(s, _)| s).expect("dag some");

    assert_eq!(hot.as_str(), std::str::from_utf8(content).unwrap());
    assert_eq!(arc.as_str(), std::str::from_utf8(content).unwrap());
    assert_eq!(dag.as_str(), std::str::from_utf8(content).unwrap());
}

/// Auto-promotion should also write filesystem archive objects under .cogniv/archive/<cid>
#[test]
fn commands_auto_promotion_writes_archive_objects() {
    let cmds = Commands::new("ignored", None).expect("commands new");

    // Use a unique lobe to avoid interference across tests
    let ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let lobe = format!("arch_e2e_{}", ns);

    // Ingest 5 items to trigger auto-promotion path in Commands::remember
    let mut contents: Vec<String> = Vec::new();
    for i in 1..=5 {
        let text = format!("archive test note {}", i);
        contents.push(text.clone());
        let _ = cmds
            .remember(&lobe, Some(&format!("k{}", i)), &text)
            .expect("remember");
    }

    // Resolve archive dir from init and assert the CIDs exist as files
    let report = ensure_initialized_once().expect("init");
    let arch_dir = report.root.join("archive");
    assert!(arch_dir.exists(), ".cogniv/archive should exist");

    for text in contents {
        let cid = blake3::hash(text.as_bytes()).to_hex().to_string();
        let p = arch_dir.join(&cid);
        assert!(
            p.exists(),
            "expected archive object missing: {:?}",
            p
        );
    }
}

/// One recall path should "heal" missing tiers: starting from hot only,
/// archive and DAG recalls should succeed and populate their pointers.
#[test]
fn commands_recall_heals_and_returns_all_tiers() {
    let cmds = Commands::new("ignored", None).expect("commands new");

    // Start with only hot present
    let content = "Sarah has a limited training budget of 100 GPU hours and prefers compute-efficient solutions.";
    let id = cmds
        .remember("preferences", Some("profile_test"), content)
        .expect("remember");

    // Hot recall
    let hot = cmds
        .recall_with_source(&id, Some("hot"))
        .expect("recall hot")
        .expect("some");
    assert_eq!(hot.1, "hot");
    assert_eq!(hot.0, content);

    // Archive recall should auto-ensure archived_cid and return content
    let arch = cmds
        .recall_with_source(&id, Some("archive"))
        .expect("recall archive")
        .expect("some");
    assert_eq!(arch.1, "archive");
    assert_eq!(arch.0, content);

    // DAG recall should auto-promote this id and return content
    let dag = cmds
        .recall_with_source(&id, Some("dag"))
        .expect("recall dag")
        .expect("some");
    assert_eq!(dag.1, "dag");
    assert_eq!(dag.0, content);

    // Verify archived_cid is set correctly in DB and file exists
    let report = ensure_initialized_once().expect("init");
    let db_path = report.root.join("cache").join("memory.db");
    let conn = open_sqlite(&db_path);
    let cid: Option<String> = conn
        .query_row(
            "SELECT archived_cid FROM memories WHERE memory_id=?1",
            [id.as_str()],
            |r| r.get(0),
        )
        .ok()
        .flatten();
    let expected_cid = blake3::hash(content.as_bytes()).to_hex().to_string();
    assert_eq!(cid.as_deref(), Some(expected_cid.as_str()));
    let arch_file = report.root.join("archive").join(&expected_cid);
    assert!(arch_file.exists(), "archive blob should exist");
}

/// Pushdown order: Hot -> DAG -> Archive. Verify identical content from each tier
#[test]
fn commands_pushdown_hot_dag_archive_same_content() {
    let cmds = Commands::new("ignored", None).expect("commands new");

    let content = "Profile: Sarah prefers compute-efficient solutions and has 100 GPU hours.";
    let id = cmds
        .remember("preferences", Some("profile_pushdown"), content)
        .expect("remember");

    // Step 1: DAG — promote this id by calling DAG recall
    let dag = cmds
        .recall_with_source(&id, Some("dag"))
        .expect("recall dag")
        .expect("some");
    assert_eq!(dag.1, "dag");
    assert_eq!(dag.0, content);

    // Step 2: Archive — ensure archive exists and recall
    let arch = cmds
        .recall_with_source(&id, Some("archive"))
        .expect("recall archive")
        .expect("some");
    assert_eq!(arch.1, "archive");
    assert_eq!(arch.0, content);

    // Hot should still return the same content
    let hot = cmds
        .recall_with_source(&id, Some("hot"))
        .expect("recall hot")
        .expect("some");
    assert_eq!(hot.1, "hot");
    assert_eq!(hot.0, content);
}

/// Exact-dedupe guard: remembering identical content twice in the same lobe
/// should return the same memory_id and keep only one row.
#[test]
fn commands_remember_dedupes_exact_duplicates() {
    let cmds = Commands::new("ignored", None).expect("commands new");

    // Use a unique lobe to avoid cross-test interference
    let ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let lobe = format!("dedupe_e2e_{}", ns);
    let content = "same content for dedupe";

    let id1 = cmds
        .remember(&lobe, Some("k1"), content)
        .expect("remember first");
    let id2 = cmds
        .remember(&lobe, Some("k2"), content)
        .expect("remember duplicate");

    // Dedupe guard should return the existing id
    assert_eq!(id1, id2, "duplicate remember should return the same id");

    // Verify only one row with this exact content exists
    let report = ensure_initialized_once().expect("init");
    let db_path = report.root.join("cache").join("memory.db");
    let conn = open_sqlite(&db_path);
    let cnt: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE lobe=?1 AND content=?2",
            (&lobe, content.as_bytes()),
            |r| r.get(0),
        )
        .expect("count dup rows");
    assert_eq!(cnt, 1, "exact duplicate rows should be collapsed to 1");
}

/// Auto-prune after remember should remove pre-existing exact duplicates in the lobe.
#[test]
fn commands_auto_prune_removes_existing_duplicates() {
    // Prepare duplicates directly via Memory in the canonical DB
    let report = ensure_initialized_once().expect("init");
    let db_path = report.root.join("cache").join("memory.db");
    let mem = Memory::open(db_path.to_str().unwrap()).expect("mem open");

    let ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let lobe = format!("prune_e2e_{}", ns);
    let dup = b"DUPLICATE PAYLOAD";

    // Two rows with identical content
    let id1 = format!("{}_1", lobe);
    mem.remember(&id1, &lobe, "a", dup).expect("remember dup1");
    thread::sleep(Duration::from_millis(5));
    let id2 = format!("{}_2", lobe);
    mem.remember(&id2, &lobe, "b", dup).expect("remember dup2");

    // Sanity: we have 2 duplicates
    let conn = open_sqlite(&db_path);
    let before: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE lobe=?1 AND content=?2",
            (&lobe, dup),
            |r| r.get(0),
        )
        .expect("count before");
    assert_eq!(before, 2, "test setup should have two exact duplicates");

    // Trigger auto-prune by calling Commands::remember (any content in same lobe)
    let cmds = Commands::new("ignored", None).expect("commands new");
    let _ = cmds
        .remember(&lobe, Some("c"), "unique content to trigger prune")
        .expect("remember trigger prune");

    // After prune, only one of the duplicate rows should remain
    let after: i64 = open_sqlite(&db_path)
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE lobe=?1 AND content=?2",
            (&lobe, dup),
            |r| r.get(0),
        )
        .expect("count after");
    assert_eq!(after, 1, "auto prune should reduce duplicates to 1");
}
