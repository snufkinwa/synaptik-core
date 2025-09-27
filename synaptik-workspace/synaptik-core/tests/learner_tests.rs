use std::time::Duration;

use rusqlite::Connection;
use synaptik_core::services::learner::StepAssembler;
use synaptik_core::services::memory::Memory;

fn open_sqlite<P: AsRef<std::path::Path>>(p: P) -> Connection {
    if let Some(parent) = p.as_ref().parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    Connection::open(p).expect("open sqlite")
}

fn tmp_db(name: &str) -> std::path::PathBuf {
    let ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("synaptik_learner_{ns}_{name}.sqlite3"))
}

#[test]
fn learner_td_updates_and_is_distinct_from_remember_recall() {
    // Setup isolated SQLite
    let db_path = tmp_db("learner_one");
    let mem = Memory::open(db_path.to_str().unwrap()).expect("mem open");
    let conn = open_sqlite(&db_path);

    // Ensure learner tables exist
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS "values" (
            state_id TEXT PRIMARY KEY,
            value REAL NOT NULL,
            updated_ms INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS steps (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            lobe TEXT NOT NULL,
            state_id TEXT,
            action_capsule_id TEXT NOT NULL,
            reward REAL NOT NULL,
            next_state_id TEXT,
            ts_ms INTEGER NOT NULL
        );
        "#,
    )
    .expect("init schema");

    // 1) Pure memory I/O should not touch `values`
    let id = "chat_s1";
    mem.remember(id, "chat", "k1", b"hello").expect("remember");
    let _ = mem.recall(id).expect("recall").expect("some");
    let cnt: i64 = conn
        .query_row("SELECT COUNT(*) FROM \"values\"", [], |r| r.get(0))
        .unwrap_or(0);
    assert_eq!(cnt, 0, "remember/recall must not update learner values");

    // 2) Learner step should upsert a positive value for the state
    let asm = StepAssembler::open_at(db_path.clone()).expect("asm");
    let now_ms = chrono::Utc::now().timestamp_millis();
    asm.record_step("chat", id, "caps1", 1.0, None, now_ms)
        .expect("record_step");
    let v: f32 = conn
        .query_row(
            "SELECT value FROM \"values\" WHERE state_id=?1",
            [id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(v > 0.0, "value should increase after positive reward");
}

#[test]
fn assembler_record_from_reward_finds_next_state_in_lobe() {
    let db_path = tmp_db("assembler_sprime");
    let mem = Memory::open(db_path.to_str().unwrap()).expect("mem open");
    let conn = open_sqlite(&db_path);
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS "values" (
            state_id TEXT PRIMARY KEY,
            value REAL NOT NULL,
            updated_ms INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS steps (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            lobe TEXT NOT NULL,
            state_id TEXT,
            action_capsule_id TEXT NOT NULL,
            reward REAL NOT NULL,
            next_state_id TEXT,
            ts_ms INTEGER NOT NULL
        );
        "#,
    )
    .expect("init schema");

    // Write two states in the same lobe with distinct timestamps
    let s1 = "chat_s1";
    mem.remember(s1, "chat", "k1", b"one").expect("remember s1");
    // ensure s2 has later updated_at
    std::thread::sleep(Duration::from_millis(5));
    let s2 = "chat_s2";
    mem.remember(s2, "chat", "k2", b"two").expect("remember s2");

    // Fetch s1 updated_at → ts_ms
    let ts_rfc: String = conn
        .query_row(
            "SELECT updated_at FROM memories WHERE memory_id=?1",
            [s1],
            |r| r.get(0),
        )
        .unwrap();
    let ts_ms = (chrono::DateTime::parse_from_rfc3339(&ts_rfc)
        .unwrap()
        .with_timezone(&chrono::Utc)
        + chrono::Duration::milliseconds(1))
    .timestamp_millis();

    let asm = StepAssembler::open_at(db_path.clone()).expect("asm");
    asm.record_from_reward("chat", s1, "caps_action", 0.1, ts_ms)
        .expect("record_from_reward");

    // Validate that step captured s2 as next_state_id
    let got_next: Option<String> = conn
        .query_row(
            "SELECT next_state_id FROM steps ORDER BY id DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .ok();
    assert_eq!(got_next.as_deref(), Some(s2));
}
