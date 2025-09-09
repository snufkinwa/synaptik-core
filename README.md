

<p align="center"><img src="./images/synaptik.png" /></p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/github/license/snufkinwa/synaptik-core" alt="License"></a>
  <img src="https://img.shields.io/badge/rust-1.7+-orange?logo=rust" alt="Rust">
  <img src="https://img.shields.io/badge/python-3.8%2B-blue?logo=python" alt="Python">
  <a href="https://pypi.org/project/synaptik-core-beta/">
  <img src="https://img.shields.io/pypi/v/synaptik-core-beta.svg" alt="PyPI version">
</a>
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

## Why It Matters

LLMs are like toddlers: sponges for patterns, but unsafe without guidance.  
Just as a toddler needs a parent to cross the street, an LLM needs an outer layer that enforces safe, accountable behavior.  

Even in software, React has a **parent container** that wraps and organizes child components — without it, the system breaks down.  

**Synaptik Core is that container for AI:** durable memory, enforceable ethics, and verifiable accountability.

```md
Parent Container (Synaptik Core)
├── Memory Management
├── Ethics Enforcement
└── Child Component (LLM)
    ├── Pattern Recognition
    ├── Creative Output
    └── Reasoning
```

## Installation

Install from PyPI:

```bash
pip install synaptik-core-beta
```

View on [PyPI↗](https://pypi.org/project/synaptik-core-beta/)

- Prebuilt wheels: macOS x86_64 (11+), Linux x86_64 (manylinux2014)
- Other platforms/arches: build from source (see From Source)
- Requires Python 3.8+

## Quick Start

```python
import synaptik_core

# Initialize command surface
cmd = synaptik_core.PyCommands()

# Write a memory (lobe = logical namespace)
mid = cmd.remember("chat", "hello from synaptik")

# Read it back (returns dict with id/content/source)
print(cmd.recall(mid))

# Recent items within a lobe
print(cmd.recent("chat", n=5))

# Optional: pre-check against ethos contracts
print(cmd.precheck_text("generate a friendly reply", purpose="message_generation"))
```

## Testing

- Demo entrypoint: `demo/demo.py` (Groq chat demo)
- Uses Python bindings only (installed from PyPI)

1) Create and activate a Python environment (optional but recommended):

```bash
python -m venv .venv && source .venv/bin/activate  # or conda
```

2) Install Synaptik Core from PyPI and demo deps:

```bash
pip install -r demo/requirements.txt
```

3) Add your Groq API key in a `.env` at the project root:

```
GROQ_API_KEY=your_api_key_here
GROQ_MODEL=openai/gpt-oss-20b
```

4) Run the chat demo:

```bash
python -m demo.demo
```

### Replicate the Demo Video

To reproduce the exact flow shown in the demo video, run `:demo`. Then open and paste the prompts from:

- `demo/test_prompts_syn.txt`

Paste them phase-by-phase (Phase 1 → Phase 13) into the REPL after running `python -m demo.demo`. This will exercise:

- Persistent notes and preferences in the `preferences` lobe
- Promotion into archive and DAG with recall previews and sources
- Ethics precheck decisions (allow/allow_with_constraints/block)
- Logging into `.cogniv/logbook/` with audit trails

## Demo Session

```
You> :demo

🚀 Running scripted demo...
   Root: .cogniv
   💾 Saved preference id: preferences_8eeb19dc062a...
   📊 Chat before: total=5, archived=5
   📊 Chat after:  total=5, archived=5
   📦 Archive objects: 11 in .cogniv/archive/
      e.g.: dd4d57ae4b5ec5cae9ed968b693bcc586713adf1b1be3323b22d9dc988566c5f, dcbdbb160eaf2cebf764d34364319c6eb4450f1b0c3901fc86f4cd73d9df7b17
   🔎 Recall(auto) chat_fea71d82dc7a9... source=hot, content='demo chat note 5'
   🧩 Recall(dag)  chat_fea71d82dc7a9... source=dag, content='demo chat note 5'
   📁 preferences → User prefers short, friendly greetings
   📁 solutions   → Final answer: 42 because constraints...
   🛡️ Precheck: BLOCK (risk=High)
   📜 Ethics log tail:
      {"constraints":[],"intent_category":"metadata_access","passed":true,"reason":"No violations detected.","requires_escalation":false,"risk":"Low","timestamp":"2025-09-05T03:53:45.554744+00:00"}
      {"constraints":[],"intent_category":"memory_storage","passed":true,"reason":"No violations detected.","requires_escalation":false,"risk":"Low","timestamp":"2025-09-05T03:53:45.556726+00:00"}
      {"constraints":["encourage_conflict_resolution","avoid_violent_language","soften_language","suggest_cooldown","refuse_personal_harm_content","suggest_support_channels","offer_deescalation","reframe_nonviolent","do_not_repeat_harmful_phrases","reframe_constructive"],"intent_category":"chat_message","passed":false,"reason":"Violated 2 rule(s).","requires_escalation":true,"risk":"High","timestamp":"2025-09-05T03:53:45.558077+00:00"}
✅ Demo complete. Continue chatting!
```

### From Source (unsupported platforms)

If a prebuilt wheel is not available for your platform/architecture, build locally with maturin:

```bash
cd synaptik-workspace/synaptik-core-py
pip install maturin
maturin develop --release
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
