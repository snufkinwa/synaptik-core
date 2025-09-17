use std::fs;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::{bounded, Receiver, Sender};
use rand::{distributions::Alphanumeric, rngs::StdRng, Rng, SeedableRng};
use sysinfo::System;

use synaptik_core::commands::init::{ensure_initialized_once};
use synaptik_core::services::memory::Memory;

#[derive(Clone, Debug)]
struct BenchCfg {
    interactions_per_session: usize, // N
    parallel_sessions: usize,        // M
    lobe: String,
    key_prefix: String,
}

#[derive(Debug, Clone)]
struct Metrics {
    commit_latencies_ms: Vec<f64>,
    replay_latencies_ms: Vec<f64>,
    errors: usize,
    writes: usize,
    replays: usize,
    start: Instant,
    end: Instant,
    max_rss_mb: f64,
    avg_cpu_percent: f64,
    sqlite_size_mb: f64,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            commit_latencies_ms: Vec::new(),
            replay_latencies_ms: Vec::new(),
            errors: 0,
            writes: 0,
            replays: 0,
            start: Instant::now(),
            end: Instant::now(),
            max_rss_mb: 0.0,
            avg_cpu_percent: 0.0,
            sqlite_size_mb: 0.0,
        }
    }
}

fn pct(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() { return 0.0; }
    let rank = (p * (sorted.len() as f64 - 1.0)).clamp(0.0, sorted.len() as f64 - 1.0);
    let idx = rank.round() as usize;
    sorted[idx]
}

fn random_text(rng: &mut StdRng) -> String {
    let len = rng.gen_range(40..120);
    let s: String = rng
        .sample_iter(&Alphanumeric)
        .take(len)
        .map(char::from)
        .collect();
    format!("msg:{}", s)
}

fn sample_process_metrics(sys: &mut System) -> (f64, f64) {
    sys.refresh_processes();
    let pid = sysinfo::Pid::from_u32(std::process::id());
    if let Some(p) = sys.process(pid) {
        let rss_mb = p.memory() as f64 / (1024.0 * 1024.0);
        let cpu = p.cpu_usage() as f64; // 0..100 per core aggregated
        (rss_mb, cpu)
    } else {
        (0.0, 0.0)
    }
}

fn run_bench(cfg: BenchCfg) -> anyhow::Result<Metrics> {
    // Isolate to a temp working directory so .cogniv lives here
    let tmp = tempfile::tempdir()?;
    std::env::set_current_dir(tmp.path())?;

    // Ensure filesystem layout and config exist
    let init = ensure_initialized_once()?;

    // Open Memory (SQLite) in this isolated root
    let db_path = init.config.memory.cache_path.to_string_lossy().to_string();
    let base_sqlite_size_mb = fs::metadata(&init.config.memory.cache_path)
        .map(|m| m.len() as f64 / (1024.0 * 1024.0))
        .unwrap_or(0.0);

    // Single-writer queue, to respect Memory's one-writer design
    #[derive(Debug, Clone)]
    enum Op {
        Remember { id: String, key: String, data: Vec<u8>, ack: Sender<()> },
        Promote { id: String, ack: Sender<()> },
        Replay { hash: String, ack: Sender<()> },
        Stop,
    }

    let (tx, rx): (Sender<Op>, Receiver<Op>) = bounded(2048);
    let lobe_for_writer = cfg.lobe.clone();
    let writer_handle = thread::spawn(move || -> anyhow::Result<()> {
        let mem = Memory::open(&db_path)?;
        loop {
            match rx.recv() {
                Ok(Op::Remember { id, key, data, ack }) => {
                    mem.remember(&id, &lobe_for_writer, &key, &data)?;
                    let _ = ack.send(());
                }
                Ok(Op::Promote { id, ack }) => {
                    let _ = mem.promote_to_dag(&id);
                    let _ = ack.send(());
                }
                Ok(Op::Replay { hash, ack }) => {
                    let _ = mem.recall_snapshot(&hash);
                    let _ = ack.send(());
                }
                Ok(Op::Stop) | Err(_) => break,
            }
        }
        Ok(())
    });

    // Worker threads generate interactions and measure latencies for commit+replay
    let mut workers = Vec::new();
    let metrics = Arc::new(Mutex::new(Metrics::default()));
    let start = Instant::now();

    let key_prefix_for_workers = cfg.key_prefix.clone();
    for sidx in 0..cfg.parallel_sessions {
        let txc = tx.clone();
        let mref = Arc::clone(&metrics);
        let key_prefix = key_prefix_for_workers.clone();
        let n = cfg.interactions_per_session;
        workers.push(thread::spawn(move || {
            let mut rng = StdRng::seed_from_u64(0xC0FFEE + sidx as u64);
            let mut commit_lat = Vec::with_capacity(n);
            let mut replay_lat = Vec::with_capacity(n);
            let mut errors = 0usize;
            let mut writes = 0usize;
            let mut replays = 0usize;

            for i in 0..n {
                let id = format!("sess{}-i{}", sidx, i);
                let key = format!("{}-{}", key_prefix, sidx);
                let content = random_text(&mut rng).into_bytes();
                let content_hash = blake3::hash(&content).to_hex().to_string();

                let t0 = Instant::now();
                let (ack_r_tx, ack_r_rx) = bounded::<()>(0);
                if txc.send(Op::Remember { id: id.clone(), key, data: content, ack: ack_r_tx }).is_err() {
                    errors += 1;
                    continue;
                }
                let _ = ack_r_rx.recv();

                // promote (commit) to DAG; wait for completion
                let (ack_p_tx, ack_p_rx) = bounded::<()>(0);
                if txc.send(Op::Promote { id: id.clone(), ack: ack_p_tx }).is_err() { errors += 1; continue; }
                let _ = ack_p_rx.recv();
                let t1 = Instant::now();
                commit_lat.push((t1 - t0).as_secs_f64() * 1000.0);
                writes += 1;

                // Replay: recall by content hash we just wrote
                let t2 = Instant::now();
                let (ack_x_tx, ack_x_rx) = bounded::<()>(0);
                if txc.send(Op::Replay { hash: content_hash.clone(), ack: ack_x_tx }).is_err() { errors += 1; continue; }
                let _ = ack_x_rx.recv();
                let t3 = Instant::now();
                replay_lat.push((t3 - t2).as_secs_f64() * 1000.0);
                replays += 1;

            }

            let mut m = mref.lock().unwrap();
            m.commit_latencies_ms.extend(commit_lat);
            m.replay_latencies_ms.extend(replay_lat);
            m.errors += errors;
            m.writes += writes;
            m.replays += replays;
        }));
    }

    // Resource sampler thread
    let sampler_running = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let sampler_flag = sampler_running.clone();
    let mut sys = System::new_all();
    let sampler = thread::spawn(move || {
        let mut max_rss = 0.0f64;
        let mut cpu_sum = 0.0f64;
        let mut cpu_count = 0usize;
        while sampler_flag.load(std::sync::atomic::Ordering::Relaxed) {
            let (rss, cpu) = sample_process_metrics(&mut sys);
            if rss > max_rss { max_rss = rss; }
            cpu_sum += cpu;
            cpu_count += 1;
            thread::sleep(Duration::from_millis(50));
        }
        let avg_cpu = if cpu_count == 0 { 0.0 } else { cpu_sum / cpu_count as f64 };
        (max_rss, avg_cpu)
    });

    for h in workers { let _ = h.join(); }

    // Stop writer
    let _ = tx.send(Op::Stop);
    let _ = writer_handle.join();

    // Stop sampler and collect
    sampler_running.store(false, std::sync::atomic::Ordering::Relaxed);
    let (max_rss, avg_cpu) = sampler.join().unwrap_or((0.0, 0.0));

    let mut result = metrics.lock().unwrap().clone();
    result.start = start;
    result.end = Instant::now();
    result.max_rss_mb = max_rss;
    result.avg_cpu_percent = avg_cpu;

    // Measure SQLite file size
    let sqlite_size_mb = fs::metadata(&init.config.memory.cache_path)
        .map(|m| m.len() as f64 / (1024.0 * 1024.0))
        .unwrap_or(0.0);
    result.sqlite_size_mb = (sqlite_size_mb - base_sqlite_size_mb).max(0.0);

    // Sort latencies once for percentile calculation
    result.commit_latencies_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
    result.replay_latencies_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());

    Ok(result)
}

fn main() -> anyhow::Result<()> {
    // Load N and M overrides from env
    let n: usize = std::env::var("SYN_BENCH_N").ok().and_then(|s| s.parse().ok()).unwrap_or(1000);
    let m: usize = std::env::var("SYN_BENCH_M").ok().and_then(|s| s.parse().ok()).unwrap_or(4);
    let cfg = BenchCfg {
        interactions_per_session: n,
        parallel_sessions: m,
        lobe: "chat".to_string(),
        key_prefix: "load".to_string(),
    };

    eprintln!("Running workload: ingest+commit+replay — N={} M={}", n, m);
    let metrics = run_bench(cfg)?;

    // Compute throughput and latency percentiles
    let dur_s = (metrics.end - metrics.start).as_secs_f64();
    let total_interactions = metrics.writes as f64; // per interaction we counted one write
    let throughput = if dur_s > 0.0 { total_interactions / dur_s } else { 0.0 };

    let p50c = pct(&metrics.commit_latencies_ms, 0.50);
    let p95c = pct(&metrics.commit_latencies_ms, 0.95);
    let p99c = pct(&metrics.commit_latencies_ms, 0.99);
    let p50r = pct(&metrics.replay_latencies_ms, 0.50);
    let p95r = pct(&metrics.replay_latencies_ms, 0.95);
    let p99r = pct(&metrics.replay_latencies_ms, 0.99);

    let total_ops = (metrics.writes + metrics.replays) as f64;
    let error_rate = if total_ops > 0.0 { metrics.errors as f64 / total_ops * 100.0 } else { 0.0 };

    // Targets
    let target_tput = 5000.0 / 60.0; // 5k/min => per second
    let target_p95_commit = 40.0;
    let target_p95_replay = 60.0;
    let target_error_pct = 0.1;

    println!("--- Synaptik Core Load Bench: Ingest + Commit + Replay ---");
    println!("Throughput: {:.1} interactions/sec (target {:.1})", throughput, target_tput);
    println!("Latency commit ms: p50 {:.1} p95 {:.1} p99 {:.1} (target p95 < {:.0})", p50c, p95c, p99c, target_p95_commit);
    println!("Latency replay ms: p50 {:.1} p95 {:.1} p99 {:.1} (target p95 < {:.0})", p50r, p95r, p99r, target_p95_replay);
    println!("Resource: max RSS {:.1} MB, avg CPU {:.1}%, SQLite size +{:.1} MB", metrics.max_rss_mb, metrics.avg_cpu_percent, metrics.sqlite_size_mb);
    println!("Errors: {} ({:.3}%) (target < {:.3}%)", metrics.errors, error_rate, target_error_pct);

    // Simple SLO verdicts
    let ok_tput = throughput >= target_tput;
    let ok_commit = p95c < target_p95_commit;
    let ok_replay = p95r < target_p95_replay;
    let ok_err = error_rate < target_error_pct;
    println!("SLOs: throughput={} commit={} replay={} errors={}", ok_tput, ok_commit, ok_replay, ok_err);

    Ok(())
}
