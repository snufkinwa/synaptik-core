use std::env;
use std::fs::{self, File};
use std::io::{self, Write, BufRead};
use std::path::Path;
use uuid::Uuid;

fn main() {
    eprintln!("Cogniv system engaged...");

    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        eprintln!("Missing command. Try: cogniv init | remember | reflect | prune");
        return;
    }

    match args[1].as_str() {
        "init" => {
            if Path::new(".cogniv").exists() {
                println!("Cognitive memory space already initialized.");
                return;
            }
            fs::create_dir(".cogniv").unwrap();
            fs::create_dir(".cogniv/objects").unwrap();
            fs::create_dir(".cogniv/refs").unwrap();
            fs::create_dir(".cogniv/contracts").unwrap();
            fs::write(".cogniv/HEAD", "ref: refs/heads/main\n").unwrap();
            println!("Initialized cognitive memory space.");
        }

        "remember" => {
            println!("Enter your memory (press Enter to finish):");
            let stdin = io::stdin();
            let mut memory = String::new();
            stdin.lock().read_line(&mut memory).unwrap();

            let id = Uuid::new_v4();
            let path = format!(".cogniv/objects/{}.txt", id);
            let mut file = File::create(&path).unwrap();
            writeln!(file, "{}", memory.trim()).unwrap();
            println!("Memory saved as {}", path);
        }

        "reflect" => {
            println!("Stored memories:");
            let entries = fs::read_dir(".cogniv/objects").unwrap();
            for entry in entries {
                let path = entry.unwrap().path();
                if path.is_file() {
                    let content = fs::read_to_string(&path).unwrap_or("".to_string());
                    println!("- {:?}: {}", path.file_name().unwrap(), content);
                }
            }
        }

      "prune" => {
            let mut entries: Vec<_> = fs::read_dir(".cogniv/objects")
                .unwrap()
                .filter_map(|e| e.ok())
                .collect();

            if entries.len() <= 1 {
                println!("Not enough entries to prune.");
                return;
            }

            println!("Pruning oldest memory...");
            entries.sort_by_key(|e| e.metadata().unwrap().created().unwrap());
            let to_delete = &entries[0];
            fs::remove_file(to_delete.path()).unwrap();
            println!("Deleted {:?}", to_delete.path().file_name().unwrap());
        }


        "help" => {
            println!("Available commands:");
            println!("  init       - Initialize the cognitive memory space");
            println!("  remember   - Save a new memory node");
            println!("  reflect    - Traverse memory snapshots");
            println!("  prune <n>  - Remove the oldest n memory snapshots");
            println!("  help       - Show this help message");
        }
        unknown => {
            println!("Unknown command: {}", unknown);
        }
    }
}
