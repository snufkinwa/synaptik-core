

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
- **Python API** — `remember()`, `reflect()`, `recall()`, `stats()`
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
from synaptik_core import PyCommands

cmd = PyCommands()
mid = cmd.remember("chat", "Hello from Synaptik Core.")
print("Reflection:", cmd.reflect("chat", 20))
print("Stats:", cmd.stats())
```

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