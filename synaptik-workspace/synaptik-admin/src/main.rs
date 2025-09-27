use std::fs;
use std::path::PathBuf;
use std::process::Command as Proc;

use anyhow::{Context, Result};
use chrono::Utc;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "synaptik-admin",
    about = "Admin helpers for Synaptik contracts GitOps"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Initialize a local registry and first pack for a channel
    RegistryInit {
        #[arg(long)]
        dir: String,
        #[arg(long)]
        out: String,
        #[arg(long, default_value = "alpha")]
        channel: String,
        #[arg(long)]
        sk_hex: Option<String>,
        #[arg(long)]
        key_id: Option<String>,
    },
    /// Promote a channel version to another channel (append to registry.jsonl)
    RegistryPromote {
        #[arg(long)]
        out: String,
        #[arg(long)]
        from: String,
        #[arg(long)]
        to: String,
        #[arg(long)]
        version: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::RegistryInit {
            dir,
            out,
            channel,
            sk_hex,
            key_id,
        } => registry_init(&dir, &out, &channel, sk_hex.as_deref(), key_id.as_deref()),
        Cmd::RegistryPromote {
            out,
            from,
            to,
            version,
        } => registry_promote(&out, &from, &to, &version),
    }
}

fn registry_init(
    dir: &str,
    out: &str,
    channel: &str,
    sk_hex: Option<&str>,
    key_id: Option<&str>,
) -> Result<()> {
    let outp = PathBuf::from(out);
    let packs = outp.join("packs");
    fs::create_dir_all(&packs).with_context(|| format!("mkdir -p {packs:?}"))?;
    let pack_path = packs.join(format!("pack-{channel}-1.json"));

    // Shell out to contracts-signer (must be on PATH)
    let mut cmd = Proc::new("contracts-signer");
    cmd.arg("--dir").arg(dir).arg("--out").arg(&pack_path);
    if let Some(sk) = sk_hex {
        cmd.arg("--sk-hex").arg(sk);
    }
    if let Some(k) = key_id {
        cmd.arg("--key-id").arg(k);
    }
    let status = cmd.status().context("contracts-signer execute")?;
    anyhow::ensure!(status.success(), "contracts-signer failed");

    let reg = outp.join("registry.jsonl");
    let ev = serde_json::json!({
        "t": Utc::now().to_rfc3339(),
        "op": "PUBLISH",
        "channel": channel,
        "uri": format!("file://{}", pack_path.display()),
    });
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&reg)
        .and_then(|mut f| {
            std::io::Write::write_all(&mut f, format!("{ev}\n").as_bytes())
        })
        .with_context(|| format!("append registry event to {:?}", reg))?;
    println!("initialized registry at {}", reg.display());
    Ok(())
}

fn registry_promote(out: &str, from: &str, to: &str, version: &str) -> Result<()> {
    let reg = PathBuf::from(out).join("registry.jsonl");
    anyhow::ensure!(reg.exists(), "registry.jsonl missing at {}", reg.display());
    let ev = serde_json::json!({
        "t": Utc::now().to_rfc3339(),
        "op": "PROMOTE",
        "from": from,
        "to": to,
        "version": version,
    });
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&reg)
        .and_then(|mut f| {
            std::io::Write::write_all(&mut f, format!("{ev}\n").as_bytes())
        })
        .context("append registry")?;
    println!("promoted {from} -> {to} version {version}");
    Ok(())
}
