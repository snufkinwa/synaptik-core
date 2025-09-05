

<p align="center"><img src="./images/synaptik.png" /></p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/github/license/snufkinwa/synaptik-core" alt="License"></a>
  <img src="https://img.shields.io/badge/rust-1.8+-orange?logo=rust" alt="Rust">
  <img src="https://img.shields.io/badge/python-3.8%2B-blue?logo=python" alt="Python">
  <img src="https://img.shields.io/badge/OpenAI-Hackathon-ff69b4?logo=openai" alt="Hackathon">
</p>

**AI Symbiosis, not just automation.**

Lightweight Rust/Python kernel that gives LLM agents durable memory and auditable ethics, stored locally in `.cogniv/`.

## Features

- **Local-first workspace** — SQLite cache + content-addressed file storage
- **Persistent memory** with automatic summarization and reflection  
- **Built-in ethics** via TOML contracts and audit trails
- **Python API (MemoryBridge)** — `root()`, `stats()`, `remember()`, `reflect()`, `recent()`, `recall()`, `get()`, `recall_many()`
- **No cloud dependency** — everything runs locally

## Quick Start

```bash
# Install (requires Rust + Python 3.8+)
cd synaptik-workspace/synaptik-core-py
pip install maturin
maturin develop --release
```

### Development Environment
- Built and tested on macOS with Miniconda
- Uses `abi3-py38` stable ABI for Python 3.8+ compatibility
- Cross-platform support via PyO3 + maturin toolchain
- Wheel installs directly to active conda Python environment
- **Note**: Not tested with isolated requirements.txt installation

```python
# Ergonomic wrapper around PyCommands with unified recall
from memory_bridge import MemoryBridge

mem = MemoryBridge()

# Basic ops
mid = mem.remember("chat", "Hello from Synaptik Core.")
print("Root:", mem.root())
print("Stats:", mem.stats())
print("Reflect:", mem.reflect("chat", 20))
print("Recent IDs:", mem.recent("chat", 3))

# Unified recall variants
print("Recall (dict):", mem.recall(mid))                 # {"content": str, "source": "hot|archive|dag"}
print("Content only:", mem.get(mid, "hot"))             # str | None (preferred tier optional)
print("Batch recall:", mem.recall_many([mid]))          # list[dict]
```

### MemoryBridge API
- `root()`: Returns workspace root path.
- `stats(lobe: Optional[str] = None)`: Returns counts by lobe and totals.
- `remember(lobe: str, content: str, key: Optional[str] = None)`: Stores and returns Memory ID.
- `reflect(lobe: str, window: int)`: Summarizes recent content for a lobe.
- `recent(lobe: str, n: int = 10)`: Returns recent Memory IDs.
- `recall(memory_id: str, prefer: Optional[str] = None)`: `{content, source}` or `None`.
- `get(memory_id: str, prefer: Optional[str] = None)`: Content string or `None`.
- `recall_many(memory_ids: list[str], prefer: Optional[str] = None)`: Batch recall.
 - Convenience: `recent_with_content(lobe, n=3, prefer=None)`, `print_recall_preview(memory_id, prefer=None, width=80)`.

## Chat Demo

Try the Groq-powered demo:

```bash
# Set GROQ_API_KEY in .env
python -m demo.demo
```

Features persistent memory, ethics checking, and tool integration.

## Demo Session

```
You> :demo

🚀 Running scripted demo...
   Root: .cogniv
   💾 Saved preference id: preferences_8eeb19dc062a...
   📊 Chat before: total=0, archived=0
   📊 Chat after:  total=5, archived=5
   🔎 Recall(auto) chat_110c24a6557e2... source=hot, content='demo chat note 5'
   🧩 Recall(dag)  chat_110c24a6557e2... source=dag, content='demo chat note 5'
   📁 preferences → User prefers short, friendly greetings
   📁 solutions   → Final answer: 42 because constraints...
   🛡️ Precheck: BLOCK (risk=High)
   📜 Ethics log tail:
      {"constraints":[],"intent_category":"metadata_access","passed":true...}
✅ Demo complete. Continue chatting!
```

## Architecture

```
Prompt → Ethics Check → Commands → Memory (SQLite hot cache) 
       → Archivist (File archive, CIDs) → DAG (immutable history) → Audit Logs
```

- **Hot cache**: SQLite for fast recall
- **Cold storage**: Content-addressed files (BLAKE3)  
- **Ethics**: TOML contracts with audit trails
- **Reflection**: TF-IDF keyword analysis

## License

Apache License 2.0

---

*Intelligence without memory is reactive. Intelligence without ethics is dangerous. Synaptik Core provides both.*
