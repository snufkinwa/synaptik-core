// tests/e2e.rs
// End-to-end tests for Synaptik MVP: Memory + Librarian + Archivist + Commands + LobeStore
//
// Run with: cargo test -- --nocapture

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use blake3;
use std::{thread, time::Duration};

use rusqlite::Connection;

use synaptik_core::commands::Commands;
use synaptik_core::services::archivist::Archivist;
use synaptik_core::services::librarian::Librarian;
use synaptik_core::services::lobes::LobeStore;
use synaptik_core::services::memory::Memory;

static COUNTER: AtomicU64 = AtomicU64::new(0);

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
    // Arrange
    let db_path = tmp_path("mem.sqlite3");
    let cmds = Commands::new(db_path.to_str().unwrap(), None).expect("commands new");

    // Act: remember (always summarizes)
    let content = "User prefers concise explanations. They like short answers. This is a test.";
    let memory_id = cmds.remember("chat", None, content).expect("remember");

    // Act: reflect over recent summaries for this lobe
    let note = cmds.reflect("chat", 20).expect("reflect");
    assert!(note.len() <= 256, "reflection should be short");

    // Act: stats
    let stats = cmds.stats(None).expect("stats");
    assert!(stats.total >= 1, "at least one row should exist");
    assert!(stats.last_updated.is_some());

    // Assert: reflection column exists for this row
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
fn lobes_put_get_list_latest() {
    // Arrange
    let root = tmp_path("lobes_root");
    let store = LobeStore::open(&root).expect("open lobe store");
    store.create_lobe("vision").expect("create lobe");

    // Act: put an object
    let key = "run42/frame_000001.png";
    let bytes = b"PNG\x89small";
    let (ver, etag, _path) = store.put_object("vision", key, bytes).expect("put");
    assert!(!ver.is_empty());
    assert!(!etag.is_empty());

    // Act: get latest
    let got = store.get_object_latest("vision", key).expect("get latest");
    assert_eq!(got, bytes);

    // Act: list latest (prefix = run42/)
    let rows = store.list_latest("vision", Some("run42/"), 10).expect("list");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, "run42/frame_000001.png");
    assert_eq!(rows[0].1, ver);
    assert_eq!(rows[0].2 as usize, bytes.len());
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
        let (cid, at): (Option<String>, Option<String>) = conn.query_row(
            "SELECT archived_cid, archived_at FROM memories WHERE memory_id=?1",
            [id.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        ).expect("row exists");
        assert_eq!(cid.as_deref(), Some(exp_cid.as_str()));
        assert!(at.is_some(), "archived_at should be set for {}", id);
    }

    // Re-promoting should do nothing (already archived)
    let again = mem.promote_all_hot_in_lobe("chat").expect("promote_all again");
    assert!(again.is_empty(), "no hot rows left to promote");
}

#[test]
fn memory_promote_latest_hot_in_lobe_single() {
    // Arrange
    let db_path = tmp_path("mem2.sqlite3");
    let mem = Memory::open(db_path.to_str().unwrap()).expect("mem open");

    // One archived row and one still hot
    mem.remember("chat_a1", "chat", "a1", b"old").expect("remember a1");
    mem.promote_all_hot_in_lobe("chat").expect("promote a1"); // archive the first

    // new hot row
    mem.remember("chat_a2", "chat", "a2", b"new").expect("remember a2");

    // Act: promote only the most recent hot row
    let one = mem.promote_latest_hot_in_lobe("chat").expect("promote latest");
    assert!(one.is_some(), "should promote one row");
    let (id, cid) = one.unwrap();

    assert_eq!(id, "chat_a2");
    assert_eq!(cid, blake3::hash(b"new").to_hex().to_string());

    // Next call should find nothing hot
    let none = mem.promote_latest_hot_in_lobe("chat").expect("promote latest none");
    assert!(none.is_none());
}
