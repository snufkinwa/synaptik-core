// examples/mvp.rs
// Minimal CLI for Synaptik MVP: remember / reflect / stats
//
// Build/run:
//   cargo run --example mvp -- --db ./data/memory.sqlite3 remember notes "User prefers concise explanations"
//   cargo run --example mvp -- --db ./data/memory.sqlite3 reflect notes 20
//   cargo run --example mvp -- --db ./data/memory.sqlite3 stats
//
// Optional cold archive (files + index DB):
//   cargo run --example mvp -- \
//     --db ./data/memory.sqlite3 \
//     --archive-root ./.cogniv/archive \
//     remember notes "Long content that should be promoted later"
// (Archivist is constructed but promotion is controlled in Librarian/Commands; keep off for MVP.)

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use synaptik_core::commands::Commands;
use synaptik_core::services::archivist::Archivist;

fn usage() -> ! {
    eprintln!(
        "Synaptik MVP CLI

USAGE:
  mvp --db <PATH> remember <lobe> [--key <key>] <content>
  mvp --db <PATH> reflect  <lobe> [window]
  mvp --db <PATH> stats    [--lobe <lobe>]

GLOBAL OPTIONS:
  --db <PATH>              SQLite file for Memory (required)
  --archive-root <DIR>     Optional: directory where Archivist stores CIDs (files)
  --archive-index <PATH>   Optional: SQLite file for Archivist index (defaults to <archive-root>/archive_index.sqlite3)

NOTES:
  - <content> can be '-' to read from STDIN.
  - If --key is omitted for 'remember', a timestamped key is generated.
"
    );
    std::process::exit(2);
}

fn main() -> ExitCode {
    let mut args = env::args().skip(1).collect::<Vec<_>>();
    if args.is_empty() {
        usage();
    }

    // ---- global flags ----
    let mut db_path: Option<String> = None;
    let mut archive_root: Option<PathBuf> = None;
    let mut archive_index: Option<PathBuf> = None;

    // Pull out global flags (consume pairs)
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--db" if i + 1 < args.len() => {
                db_path = Some(args.remove(i + 1));
                args.remove(i);
            }
            "--archive-root" if i + 1 < args.len() => {
                archive_root = Some(PathBuf::from(args.remove(i + 1)));
                args.remove(i);
            }
            "--archive-index" if i + 1 < args.len() => {
                archive_index = Some(PathBuf::from(args.remove(i + 1)));
                args.remove(i);
            }
            _ => i += 1,
        }
    }

    let db_path = match db_path {
        Some(p) => p,
        None => {
            eprintln!("error: --db <PATH> is required\n");
            usage();
        }
    };

    if args.is_empty() {
        usage();
    }
    let cmd = args.remove(0);

    // Optional Archivist wiring
    let archivist = match archive_root {
        Some(root) => {
            // Ensure root exists
            if let Err(e) = std::fs::create_dir_all(&root) {
                eprintln!("error: creating archive-root {:?}: {}", root, e);
                return ExitCode::from(1);
            }
            // Choose index path
            let idx_path = archive_index.unwrap_or_else(|| root.join("archive_index.sqlite3"));
            // Open SQLite for index
            let arch_db = match rusqlite::Connection::open(&idx_path) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("error: opening archive index DB {:?}: {}", idx_path, e);
                    return ExitCode::from(1);
                }
            };
            match Archivist::open(root, arch_db) {
                Ok(a) => Some(a),
                Err(e) => {
                    eprintln!("error: initializing Archivist: {e}");
                    return ExitCode::from(1);
                }
            }
        }
        None => None,
    };

    // Build Commands (single-writer Memory inside)
    let cmds = match Commands::new(&db_path, archivist) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: commands init: {e}");
            return ExitCode::from(1);
        }
    };

    // ---- subcommands ----
    match cmd.as_str() {
        "remember" => {
            // remember <lobe> [--key <key>] <content>
            if args.len() < 2 {
                eprintln!("error: remember requires <lobe> <content> (with optional --key <key>)");
                return ExitCode::from(2);
            }
            // parse optional --key
            let mut lobe = args.remove(0);
            let mut key: Option<String> = None;

            // scan for "--key <val>" among remaining args except the last (content)
            let mut j = 0;
            while j + 1 < args.len() {
                if args[j] == "--key" {
                    key = Some(args.remove(j + 1));
                    args.remove(j);
                } else {
                    j += 1;
                }
            }

            if args.is_empty() {
                eprintln!("error: missing <content>");
                return ExitCode::from(2);
            }
            let content_arg = args.remove(0);
            let content = if content_arg == "-" {
                // read stdin
                let mut s = String::new();
                if let Err(e) = std::io::Read::read_to_string(&mut std::io::stdin(), &mut s) {
                    eprintln!("error: reading stdin: {e}");
                    return ExitCode::from(1);
                }
                s
            } else if content_arg.starts_with("@") {
                // read file after '@'
                let p = &content_arg[1..];
                match fs::read_to_string(p) {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("error: reading file {}: {}", p, e);
                        return ExitCode::from(1);
                    }
                }
            } else {
                content_arg
            };

            match cmds.remember(&lobe, key.as_deref(), &content) {
                Ok(id) => {
                    println!("{}", id);
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("error: remember: {e}");
                    ExitCode::from(1)
                }
            }
        }

        "reflect" => {
            // reflect <lobe> [window]
            if args.is_empty() {
                eprintln!("error: reflect requires <lobe> [window]");
                return ExitCode::from(2);
            }
            let lobe = args.remove(0);
            let window = args
                .get(0)
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(20);

            match cmds.reflect(&lobe, window) {
                Ok(note) => {
                    println!("{}", note);
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("error: reflect: {e}");
                    ExitCode::from(1)
                }
            }
        }

        "stats" => {
            // stats [--lobe <lobe>]
            let mut lobe: Option<String> = None;
            let mut k = 0;
            while k + 1 <= args.len() {
                if k + 1 < args.len() && args[k] == "--lobe" {
                    lobe = Some(args.remove(k + 1));
                    args.remove(k);
                } else {
                    k += 1;
                }
            }
            match cmds.stats(lobe.as_deref()) {
                Ok(s) => {
                    // lightweight JSON-ish print without adding serde_json here
                    println!("{{\"total\":{},\"archived\":{},\"by_lobe\":{:?},\"last_updated\":{:?}}}",
                        s.total, s.archived, s.by_lobe, s.last_updated);
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("error: stats: {e}");
                    ExitCode::from(1)
                }
            }
        }

        _ => {
            eprintln!("error: unknown subcommand '{}'\n", cmd);
            usage();
        }
    }
}
