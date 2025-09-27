use std::fs;
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use blake3;
use chrono::Utc;
use clap::Parser;
use ed25519_dalek::{SigningKey, Signature, Signer};
use serde::{Deserialize, Serialize};
use serde_json::json;
use walkdir::WalkDir;
use toml_edit::DocumentMut;

#[derive(Parser, Debug)]
#[command(name = "contracts-signer", about = "Build and sign a Synaptik contracts pack")] 
struct Cli {
    /// Directory containing contract files (TOML)
    #[arg(long, value_name = "DIR")]
    dir: String,
    /// Output JSON file (pack)
    #[arg(long, value_name = "FILE")]
    out: String,
    /// Ed25519 secret key hex (optional). If omitted, pack is unsigned.
    #[arg(long)]
    sk_hex: Option<String>,
    /// Signing key id label stored in the pack
    #[arg(long)]
    key_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PackFileEntry { path: String, blake3: String, size: u64 }

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ContractPack {
    version: String,
    algo: String,
    canon_hash: String,
    files: Vec<PackFileEntry>,
    blobs: std::collections::BTreeMap<String, String>,
    policy: serde_json::Value,
    #[serde(default)]
    signature: Option<String>,
    #[serde(default)]
    signing_key_id: Option<String>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let base = PathBuf::from(&cli.dir);
    anyhow::ensure!(base.exists() && base.is_dir(), "dir must exist and be a directory");

    // Collect TOML files under dir
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for e in WalkDir::new(&base).into_iter().filter_map(|e| e.ok()) {
        let p = e.path();
        if p.is_file() && p.extension().and_then(|s| s.to_str()) == Some("toml") {
            let rel = pathdiff::diff_paths(p, &base).unwrap_or_else(|| p.to_path_buf());
            let rel_s = rel.to_string_lossy().to_string();
            let raw = fs::read_to_string(p).with_context(|| format!("read {:?}", p))?;
            let normalized = raw.parse::<DocumentMut>().map(|d| d.to_string()).unwrap_or(raw);
            files.push((rel_s, normalized.into_bytes()));
        }
    }

    // Build manifest
    files.sort_by(|a,b| a.0.cmp(&b.0));
    let list: Vec<PackFileEntry> = files.iter().map(|(rel, bytes)| {
        PackFileEntry { path: format!("contracts/{}", rel), blake3: blake3::hash(bytes).to_hex().to_string(), size: bytes.len() as u64 }
    }).collect();

    // Compute canon hash over concatenated file hashes (sorted)
    let mut concat = String::new();
    for f in &list { concat.push_str(&f.blake3); }
    let canon_hash = blake3::hash(concat.as_bytes()).to_hex().to_string();

    let mut blobs = std::collections::BTreeMap::new();
    for (rel, bytes) in files.iter() {
        blobs.insert(format!("contracts/{}", rel), B64.encode(bytes));
    }

    let mut pack = ContractPack {
        version: Utc::now().to_rfc3339(),
        algo: "ed25519".into(),
        canon_hash,
        files: list,
        blobs,
        policy: json!({}),
        signature: None,
        signing_key_id: cli.key_id.clone(),
    };

    if let Some(skhex) = cli.sk_hex.as_deref() {
        let sk_bytes = hex::decode(skhex.trim_start_matches("ed25519:"))?;
        let sk = SigningKey::from_bytes(&sk_bytes.try_into().map_err(|_| anyhow!("bad sk length"))?);
        let mut tmp = pack.clone();
        tmp.signature = None;
        let msg = serde_json::to_vec(&tmp)?;
        let sig: Signature = sk.sign(&msg);
        pack.signature = Some(B64.encode(sig.to_bytes()));
    }

    fs::write(&cli.out, serde_json::to_vec_pretty(&pack)?).with_context(|| format!("write {}", &cli.out))?;
    Ok(())
}
