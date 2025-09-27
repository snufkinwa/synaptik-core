use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use blake3;
use chrono::Utc;
use clap::Parser;
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use toml_edit::DocumentMut;
use uuid::Uuid;

use syn_core::commands::Commands;
use synaptik_core as syn_core;

#[derive(Debug, Clone, Deserialize)]
struct AgentConfig {
    roots: Vec<String>,
    registry_uri: String, // file path or file:// URL to registry.jsonl OR directly to a pack.json
    #[serde(default)]
    trusted_pubkey_hex: Option<String>,
    #[serde(default)]
    trust_store: Option<String>, // optional TOML file with [[keys]]
    #[serde(default)]
    interval_seconds: Option<u64>,
    #[serde(default)]
    allow_reinstatement_if_bind_fails: Option<bool>,
    #[serde(default)]
    channel: Option<String>,
    #[serde(default)]
    dry_run: Option<bool>,
    #[serde(default)]
    require_locked: Option<bool>,
    #[serde(default)]
    max_atp_per_apply: Option<u64>,
    #[serde(default)]
    max_atp_per_day: Option<u64>,
}

#[derive(Parser, Debug)]
#[command(
    name = "synaptik-agent",
    about = "Synaptik contracts auto-updater agent"
)]
struct Cli {
    #[arg(long, default_value = "/etc/synaptik-agent.toml")]
    config: String,
    #[arg(long, help = "Run one cycle then exit")]
    once: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PackFileEntry {
    path: String,
    blake3: String,
    #[serde(default)]
    size: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ContractPack {
    version: String,
    #[serde(default)]
    algo: Option<String>,
    #[serde(default)]
    canon_hash: Option<String>,
    files: Vec<PackFileEntry>,
    #[serde(default)]
    blobs: BTreeMap<String, String>, // path -> base64 content
    #[serde(default)]
    policy: Option<Value>,
    #[serde(default)]
    signature: Option<String>, // base64
    #[serde(default)]
    signing_key_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct TrustKey {
    key_id: String,
    pubkey_hex: String,
    #[serde(default)]
    not_before: Option<String>,
    #[serde(default)]
    not_after: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct TrustStore {
    #[serde(default)]
    keys: Vec<TrustKey>,
}

#[derive(Debug, Clone, Deserialize)]
struct RegistryEvent {
    #[allow(dead_code)]
    t: Option<String>,
    op: String,
    #[serde(default)]
    channel: Option<String>,
    #[serde(default)]
    uri: Option<String>,
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    to: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    key_id: Option<String>,
    #[serde(default)]
    uuid: Option<String>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let cfg_text =
        fs::read_to_string(&cli.config).with_context(|| format!("read config {}", &cli.config))?;
    let cfg: AgentConfig = toml::from_str(&cfg_text).context("parse agent config")?;

    if cli.once {
        run_once(&cfg).context("run_once")?;
        return Ok(());
    }

    let interval = cfg.interval_seconds.unwrap_or(900);
    loop {
        let _ = run_once(&cfg).map_err(|e| eprintln!("[agent] cycle error: {e}"));
        thread::sleep(Duration::from_secs(interval));
    }
}

fn run_once(cfg: &AgentConfig) -> Result<()> {
    let (pack_path, revoked_keys, revoked_root_uuids) =
        resolve_pack_uri(&cfg.registry_uri, cfg.channel.as_deref())?
            .ok_or_else(|| anyhow!("no pack uri found in registry: {}", cfg.registry_uri))?;
    let pack = load_pack(&pack_path)?;
    verify_pack(&pack, cfg, &revoked_keys)?;

    for root in &cfg.roots {
        let root_path = PathBuf::from(root);
        if !root_path.exists() {
            eprintln!("[agent] root missing: {}", root);
            continue;
        }
        let dry = cfg.dry_run.unwrap_or(false);
        let ruuid = ensure_root_uuid(&root_path).unwrap_or_else(|_| "".into());
        if revoked_root_uuids.contains(&ruuid) {
            emit_status(
                &root_path,
                &pack.version,
                "REVOKED_ROOT",
                false,
                None,
                0,
                cfg.channel.as_deref(),
            );
            continue;
        }
        if let Err(e) = process_root(&root_path, &pack, cfg, &revoked_keys, dry) {
            eprintln!("[agent] root {} error: {e}", root_path.display());
        }
    }
    Ok(())
}

fn resolve_pack_uri(
    registry_uri: &str,
    channel: Option<&str>,
) -> Result<Option<(String, HashSet<String>, HashSet<String>)>> {
    // If registry_uri points directly to a JSON file that looks like a pack, return it.
    if registry_uri.ends_with(".json")
        && Path::new(registry_uri.trim_start_matches("file://")).exists()
    {
        return Ok(Some((
            registry_uri.trim_start_matches("file://").to_string(),
            HashSet::new(),
            HashSet::new(),
        )));
    }
    // Else treat as a registry.jsonl file path (file:// optional).
    let path = registry_uri.trim_start_matches("file://");
    let bytes = fs::read(path).with_context(|| format!("read registry {path}"))?;
    let text = String::from_utf8_lossy(&bytes);
    let mut revoked_keys: HashSet<String> = HashSet::new(); // key ids
    let mut revoked_roots: HashSet<String> = HashSet::new(); // root uuids
    let mut deprecated_versions: HashSet<String> = HashSet::new();
    let mut version_uri: HashMap<String, String> = HashMap::new();
    let mut events: Vec<RegistryEvent> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(ev) = serde_json::from_str::<RegistryEvent>(line) {
            events.push(ev);
        }
    }
    // Candidate per channel: (uri, version)
    let mut chan_uri: HashMap<String, (String, Option<String>)> = HashMap::new();
    for ev in &events {
        match ev.op.as_str() {
            "PUBLISH" => {
                if let (Some(ch), Some(uri)) = (ev.channel.as_ref(), ev.uri.as_ref()) {
                    chan_uri.insert(ch.clone(), (uri.clone(), ev.version.clone()));
                    if let Some(ver) = ev.version.as_ref() {
                        version_uri.insert(ver.clone(), uri.clone());
                    }
                }
            }
            "PROMOTE" => {
                if let (Some(from), Some(to)) = (ev.from.as_ref(), ev.to.as_ref()) {
                    if let Some(ver) = ev.version.as_ref() {
                        if let Some(uri) = version_uri.get(ver).cloned() {
                            chan_uri.insert(to.clone(), (uri, Some(ver.clone())));
                        }
                    } else if let Some((uri, ver)) = chan_uri.get(from).cloned() {
                        chan_uri.insert(to.clone(), (uri, ver));
                    }
                }
            }
            "REVOKE_KEY" => {
                if let Some(k) = ev.key_id.as_ref() {
                    revoked_keys.insert(k.clone());
                }
            }
            "REVOKE_ROOT" => {
                if let Some(u) = ev.uuid.as_ref() {
                    revoked_roots.insert(u.clone());
                }
            }
            "DEPRECATE_VERSION" => {
                if let Some(v) = ev.version.as_ref() {
                    deprecated_versions.insert(v.clone());
                }
            }
            _ => {}
        }
    }
    let ch = channel.unwrap_or("stable");
    if let Some((uri, ver)) = chan_uri.get(ch).cloned() {
        if let Some(v) = ver {
            if deprecated_versions.contains(&v) {
                return Ok(None);
            }
        }
        return Ok(Some((
            uri.trim_start_matches("file://").to_string(),
            revoked_keys,
            revoked_roots,
        )));
    }
    Ok(None)
}

fn load_pack(path: &str) -> Result<ContractPack> {
    let p = path.trim_start_matches("file://");
    let bytes = fs::read(p).with_context(|| format!("read pack {p}"))?;
    let pack: ContractPack = serde_json::from_slice(&bytes).context("parse pack json")?;
    Ok(pack)
}

fn verify_pack(pack: &ContractPack, cfg: &AgentConfig, revoked: &HashSet<String>) -> Result<()> {
    // Hash verification for blobs
    for f in &pack.files {
        if let Some(b64) = pack.blobs.get(&f.path) {
            let bytes = B64.decode(b64.as_bytes()).context("decode blob b64")?;
            let hash = blake3::hash(&bytes).to_hex().to_string();
            anyhow::ensure!(hash == f.blake3, "blob hash mismatch for {}", f.path);
        }
    }
    // Optional signature verification
    if let Some(sig_b64) = &pack.signature {
        let mut pack_clone = pack.clone();
        pack_clone.signature = None;
        let msg = serde_json::to_vec(&pack_clone).context("serialize pack for verify")?;
        let sig_bytes = B64
            .decode(sig_b64.as_bytes())
            .context("decode signature b64")?;
        let sig = Signature::from_slice(&sig_bytes).map_err(|e| anyhow!("signature: {e}"))?;

        // Load trust store or single key
        let mut vkeys: Vec<(String, VerifyingKey)> = Vec::new();
        if let Some(store_path) = cfg.trust_store.as_deref() {
            if Path::new(store_path).exists() {
                let tbytes = fs::read(store_path).context("read trust_store")?;
                let store: TrustStore = toml::from_str(&String::from_utf8_lossy(&tbytes))
                    .context("parse trust_store")?;
                let kid_filter = pack.signing_key_id.as_deref();
                for k in store.keys {
                    // key id filter when present
                    if let Some(kid) = kid_filter {
                        if k.key_id != kid {
                            continue;
                        }
                    }
                    // validity window check (RFC3339); if invalid or out of window -> skip
                    if let Some(nb) = &k.not_before {
                        match chrono::DateTime::parse_from_rfc3339(nb) {
                            Ok(dt) => {
                                if Utc::now() < dt.with_timezone(&Utc) {
                                    continue;
                                }
                            }
                            Err(_) => continue,
                        }
                    }
                    if let Some(na) = &k.not_after {
                        match chrono::DateTime::parse_from_rfc3339(na) {
                            Ok(dt) => {
                                if Utc::now() > dt.with_timezone(&Utc) {
                                    continue;
                                }
                            }
                            Err(_) => continue,
                        }
                    }
                    let pk_vec = hex::decode(k.pubkey_hex.trim_start_matches("ed25519:"))
                        .context("decode pubkey hex")?;
                    let pk: [u8; 32] = pk_vec
                        .try_into()
                        .map_err(|_| anyhow!("pubkey length != 32"))?;
                    let vk = VerifyingKey::from_bytes(&pk).map_err(|e| anyhow!("pubkey: {e}"))?;
                    vkeys.push((k.key_id, vk));
                }
            }
        }
        if vkeys.is_empty() {
            if let Some(pk_hex) = cfg.trusted_pubkey_hex.as_deref() {
                let pk_vec = hex::decode(pk_hex.trim_start_matches("ed25519:"))
                    .context("decode pubkey hex")?;
                let pk: [u8; 32] = pk_vec
                    .try_into()
                    .map_err(|_| anyhow!("pubkey length != 32"))?;
                let vk = VerifyingKey::from_bytes(&pk).map_err(|e| anyhow!("pubkey: {e}"))?;
                vkeys.push((pack.signing_key_id.clone().unwrap_or_default(), vk));
            }
        }

        // Enforce revocation if key id present
        if let Some(kid) = &pack.signing_key_id {
            anyhow::ensure!(!revoked.contains(kid), "signing key revoked: {kid}");
        }

        let mut ok = false;
        for (_kid, vk) in vkeys {
            if vk.verify_strict(&msg, &sig).is_ok() {
                ok = true;
                break;
            }
        }
        anyhow::ensure!(ok, "no trusted key verified the pack signature");
    }
    Ok(())
}

fn process_root(
    root: &Path,
    pack: &ContractPack,
    agent_cfg: &AgentConfig,
    _revoked: &HashSet<String>,
    dry_run: bool,
) -> Result<()> {
    // Set COGNIV_ROOT so Commands operate on this root
    std::env::set_var("COGNIV_ROOT", root);
    let cmds = Commands::builder()?.build()?;
    let core_cfg = cmds.config().clone();
    let default_name = core_cfg.contracts.default_contract.clone();
    let rel = format!("contracts/{}", default_name);
    let incoming_b64 = pack
        .blobs
        .get(&rel)
        .ok_or_else(|| anyhow!("pack missing blob for {}", rel))?;
    let incoming_bytes = B64
        .decode(incoming_b64.as_bytes())
        .context("decode incoming blob")?;
    let incoming_raw = String::from_utf8(incoming_bytes).context("incoming utf8")?;
    let incoming_text = normalize_toml(&incoming_raw);

    // Preview bind
    let (merged_raw, had_conflicts) = cmds.contracts_bind_with_text(&incoming_text, false, None)?;
    let merged = normalize_toml(&merged_raw);
    // TOML parse check
    let parses = toml::from_str::<toml::Table>(&merged).is_ok();

    // Locked discipline: require local matches embedded canon (best-effort)
    if agent_cfg.require_locked.unwrap_or(true) && !local_matches_embedded(root, &default_name) {
        emit_status(
            root,
            &pack.version,
            "LOCKED",
            had_conflicts,
            None,
            0,
            agent_cfg.channel.as_deref(),
        );
        return Err(anyhow!("root appears tampered/unlocked vs embedded canon"));
    }

    if had_conflicts || !parses {
        // Emit a conflict preview on disk for inspection
        let conf_name = format!(".conflict-{}.toml", Utc::now().format("%Y%m%dT%H%M%S"));
        let conf_path = root.join("contracts").join(conf_name);
        if let Some(parent) = conf_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(&conf_path, merged.as_bytes());
        // Optionally reinstate canon
        if agent_cfg.allow_reinstatement_if_bind_fails.unwrap_or(true) {
            let _ = cmds.reinstatement();
        }
        emit_status(
            root,
            &pack.version,
            if had_conflicts {
                "CONFLICT"
            } else {
                "PARSE_FAIL"
            },
            had_conflicts,
            None,
            0,
            agent_cfg.channel.as_deref(),
        );
        return Err(anyhow!(
            "bind unsafe: conflicts={} parses={}",
            had_conflicts,
            parses
        ));
    }

    if dry_run {
        emit_status(
            root,
            &pack.version,
            "NO_UPDATE",
            false,
            Some(blake3::hash(merged.as_bytes()).to_hex().to_string()),
            0,
            agent_cfg.channel.as_deref(),
        );
        return Ok(());
    }

    // ATP metering (rough heuristic: per-line cost)
    let atp = merged.lines().count() as u64;
    if let Some(max) = agent_cfg.max_atp_per_apply {
        if atp > max {
            emit_status(
                root,
                &pack.version,
                "ATP_CAP",
                false,
                None,
                atp,
                agent_cfg.channel.as_deref(),
            );
            return Err(anyhow!("apply exceeds ATP cap: {}>{}", atp, max));
        }
    }

    // Daily ATP budget
    if let Some(day_cap) = agent_cfg.max_atp_per_day {
        let mut state = load_atp_state(root).unwrap_or_default();
        let today = Utc::now().format("%Y%m%d").to_string();
        if state.day != today {
            state.day = today;
            state.used = 0;
        }
        if state.used.saturating_add(atp) > day_cap {
            emit_status(
                root,
                &pack.version,
                "ATP_DAILY_CAP",
                false,
                None,
                atp,
                agent_cfg.channel.as_deref(),
            );
            return Err(anyhow!(
                "daily ATP cap reached: {}+{}>{}",
                state.used,
                atp,
                day_cap
            ));
        }
        state.used = state.used.saturating_add(atp);
        let _ = save_atp_state(root, &state);
    }

    // Backup current target for rollback
    let target_path = root.join("contracts").join(&default_name);
    let backup_path = root.join("contracts").join(format!(
        ".backup-{}-{}.toml",
        default_name,
        Utc::now().format("%Y%m%dT%H%M%S")
    ));
    if target_path.exists() {
        let _ = fs::copy(&target_path, &backup_path);
    }

    // Apply: unlock → bind → relock, with rollback on error
    cmds.unlock_contracts();
    let bind_res = cmds.contracts_bind_with_text(&incoming_text, true, None);
    // Always relock before handling result to avoid leaving unlocked on early return
    cmds.lock_contracts();
    let (_merged, _conf) = bind_res.map_err(|e| {
        // rollback immediate
        if backup_path.exists() {
            let _ = fs::copy(&backup_path, &target_path);
        }
        e
    })?;

    // Write receipt
    let mhash = blake3::hash(merged.as_bytes()).to_hex().to_string();
    let receipt = serde_json::json!({
        "version": pack.version,
        "target": format!("contracts/{}", default_name),
        "had_conflicts": false,
        "ts": Utc::now().to_rfc3339(),
        "actor": whoami::fallible::hostname().unwrap_or_else(|_| "agent".into()),
        "merged_hash": mhash,
    });
    let rec_path = root.join("contracts/.bind-receipt.json");
    if let Some(parent) = rec_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(&rec_path, serde_json::to_vec_pretty(&receipt)?);
    emit_status(
        root,
        &pack.version,
        "APPLIED",
        false,
        Some(mhash),
        atp,
        agent_cfg.channel.as_deref(),
    );
    Ok(())
}

fn normalize_toml(raw: &str) -> String {
    raw.parse::<DocumentMut>()
        .map(|d| d.to_string())
        .unwrap_or_else(|_| raw.to_string())
}

fn local_matches_embedded(root: &Path, default_name: &str) -> bool {
    use contracts::assets::default_contract_text;
    let local = root.join("contracts").join(default_name);
    if !local.exists() {
        return true;
    }
    let Ok(bytes) = fs::read(&local) else {
        return false;
    };
    let Ok(text) = String::from_utf8(bytes) else {
        return false;
    };
    let norm_local = normalize_toml(&text);
    if let Some(embed) = default_contract_text(default_name) {
        let norm_emb = normalize_toml(embed);
        blake3::hash(norm_local.as_bytes()) == blake3::hash(norm_emb.as_bytes())
    } else {
        true
    }
}

fn emit_status(
    root: &Path,
    pack_version: &str,
    status: &str,
    had_conflicts: bool,
    merged_hash: Option<String>,
    atp_cost: u64,
    channel: Option<&str>,
) {
    let line = serde_json::json!({
        "ts": Utc::now().to_rfc3339(),
        "host": whoami::fallible::hostname().unwrap_or_else(|_| "agent".into()),
        "root": root.display().to_string(),
        "channel": channel.unwrap_or("stable"),
        "pack_version": pack_version,
        "status": status,
        "had_conflicts": had_conflicts,
        "merged_hash": merged_hash,
        "atp_cost": atp_cost,
    });
    println!("{}", line.to_string());
}

#[derive(Default, Serialize, Deserialize)]
struct AtpState {
    day: String,
    used: u64,
}

fn atp_state_path(root: &Path) -> PathBuf {
    root.join("contracts/.atp.json")
}
fn load_atp_state(root: &Path) -> Option<AtpState> {
    let p = atp_state_path(root);
    let bytes = fs::read(p).ok()?;
    serde_json::from_slice(&bytes).ok()
}
fn save_atp_state(root: &Path, st: &AtpState) -> Result<()> {
    let p = atp_state_path(root);
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).ok();
    }
    fs::write(p, serde_json::to_vec_pretty(st)?)?;
    Ok(())
}

fn ensure_root_uuid(root: &Path) -> Result<String> {
    let p = root.join("uuid");
    if p.exists() {
        let s = String::from_utf8(fs::read(&p)?).unwrap_or_default();
        let id = s.trim();
        if !id.is_empty() {
            return Ok(id.to_string());
        }
    }
    let id = Uuid::new_v4().to_string();
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).ok();
    }
    fs::write(&p, id.as_bytes())?;
    Ok(id)
}
