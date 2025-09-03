<p align="center"><img src="./images/synaptik.png" /></p>


**AI Symbiosis, not just automation.**
Synaptik Core is a lightweight Rust/Python kernel that gives any LLM agent durable memory and auditable ethics, all stored locally in .cogniv/.


## What is `synaptik-core`?

A small Rust library (with Python bindings) that gives LLM agents durable memory and simple, deterministic reflection:

* **Local-first workspace** — `.cogniv/` with `cache/memory.db` (SQLite) + `archive/` (content-addressed files).
* **Single-writer Memory** — one SQLite handle, safe & idempotent init.
* **Archivist** — file-only cold storage by BLAKE3 CID (no DB writes).
* **Librarian** — orchestrates ingest: optional summarization for long inputs, plus a tiny keyword reflection seed.
* **Commands** — high-level API: `remember`, `reflect`, `stats`.
* **Audit / Ethos hooks** — actions are recorded to JSONL logbook; an ethos precheck gate is in place (TOML contract seeded).

This gives agents structure and recall without cloud lock-in, and keeps behavior auditable for safety and trust.


## Architecture

```
┌─────────────────┐    ┌──────────────────────────────────────┐
│   Your App      │    │             Synaptik Core            │
│   (Groq Demo)   │    │                                      │
└─────────┬───────┘    │  ┌─────────────┐  ┌───────────────┐  │
          │            │  │  Commands   │  │     Ethos     │  │
          │            │  │             │  │   (Ethics)    │  │
          │            │  │ • remember  │  │ • precheck    │  │
          │            │  │ • reflect   │  │ • contracts   │  │
          │            │  │ • stats     │  │ • audit logs  │  │
          │            │  └─────┬───────┘  └───────────────┘  │
          │            │        │                             │
          └────────────┼────────┤                             │
                       │        │                             │
                       │  ┌─────▼───────┐  ┌───────────────┐  │
                       │  │  Librarian  │  │   Archivist   │  │
                       │  │             │  │  (Cold Store) │  │
                       │  │ • summarize │  │ • CID storage │  │
                       │  │ • reflect   │  │ • blake3 hash │  │
                       │  │ • orchestr. │  │ • filesystem  │  │
                       │  └─────┬───────┘  └───────┬───────┘  │
                       │        │                   │          │
                       │  ┌─────▼───────────────────▼───────┐  │
                       │  │            Memory              │  │
                       │  │      (Single SQLite Writer)    │  │
                       │  │                                 │  │
                       │  │ • Hot cache (.cogniv/cache/)    │  │
                       │  │ • Summaries & reflections      │  │
                       │  │ • Archive pointers (CID refs)  │  │
                       │  └─────────────────────────────────┘  │
                       └──────────────────────────────────────┘

File System Layout:
.cogniv/
├── cache/memory.db        # Hot SQLite cache
├── archive/<cid>          # Cold content-addressed files  
├── logbook/              # Audit trails (JSONL)
├── contracts/            # Ethics rules (TOML)
└── config.toml          # System configuration
```

### Key Design Principles:

* Synaptik Core fuses stateless LLMs with stateful memory + ethics:
* Hot cache (SQLite) for fast recall.
* Cold archive (CID/DAG) for verifiable, immutable history.
* Ethos contracts to enforce ethical rules with audit trails.
* ⚡ Together, this makes AI not just smart but accountable, persistent, and trustworthy.

Flow:
App → Commands → Ethos precheck → Librarian (summarize/reflect) → Memory (SQLite) → Archivist (CID cold store) → Audit Logbook

## Install (Python)

Requirements: Python ≥ 3.8, Rust toolchain, `maturin`.

```bash
# Build the Python extension in editable mode
cd synaptik-workspace/synaptik-core-py
pip install maturin
maturin develop --release
```

### Quick test

```python
from synaptik_core import PyCommands

cmd = PyCommands()
print("root:", cmd.root())

# Short notes won't be summarized; use longer text to see reflection
mid = cmd.remember("chat", "Hello from Synaptik Core.")
print("memory_id:", mid)

print("reflect:", cmd.reflect("chat", 20))
print("stats:", cmd.stats(None))
```

> **Tip**: To see meaningful reflection, ingest 3–4 longer notes (>500 chars) that share repeated terms, then call `reflect("chat", window)`.

## Chat Demo (Groq) — How to Interact

We ship a runnable demo that wires Synaptik Core into a Groq-backed chat loop and shows memory + ethics in action.

Run the demo:

```bash
# Set env vars in .env: GROQ_API_KEY, GROQ_MODEL (optional)
python -m demo.demo
```

In the REPL:

- Type normal messages and watch the assistant propose actions via a JSON object on the last line. The tool router executes: remember, reflect, stats, recent, recall.
- Type demo (or :demo) to run a 3‑minute scripted flow that:
  - Saves a preference and shows persistence across sessions
  - Adds enough chat items to trigger auto‑promotion (hot → archive/DAG)
  - Prints stats before/after (archived rises)
  - Recalls a recent id with prefer="auto" and then force prefer="dag" to show DAG reads
  - Runs an ethics precheck and tails the logbook

Notes:

- Unified recall API (Python): `cmd.recall(id, prefer)` returns a dict `{content, source}` where `source` is `hot|archive|dag`. `cmd.recall_many(ids, prefer)` returns a list of dicts.
- Local precheck runs before the LLM. If the input is unsafe, the demo does NOT forward your raw text; instead it sends a safety prompt to the LLM using the constraints returned by the contract. This ensures Ethos/Audit logs are written and you still get a helpful, safe response.
- Contracts are locked by default from the Rust side; Python bindings do not expose lock/unlock.

### Environment Setup

Create a `.env` file in your project root:

```bash
GROQ_API_KEY=your_groq_api_key_here
GROQ_MODEL=openai/gpt-oss-20b
```

### Example Session

```
🧠 Synaptik Core + Groq Demo
📁 Data directory: /path/to/your/project/.cogniv
🤖 Model: openai/gpt-oss-20b

You> Remember this: Quantum computers use qubits instead of classical bits.
🔧 Action: remember
✅ Success
   Memory ID: chat_1ce81e364cb98a4f...

You> {"action":"remember","args":{"lobe":"physics","content":"Quantum mechanics is a fundamental theory in physics that describes the physical properties of nature at atomic and subatomic scales. Unlike classical physics, quantum mechanics shows that energy, momentum, and angular momentum are quantized into discrete values called quanta. This leads to phenomena like wave-particle duality and quantum superposition.","key":"quantum_basics"}}
🔧 Action: remember
✅ Success
   Memory ID: physics_d8554b66ecdf86e0...

You> {"action":"reflect","args":{"lobe":"physics","window":10}}
🔧 Action: reflect
✅ Success
   Reflection: Recurring themes: quantum(3), mechanics(2), physics(2)

You> What did I just store in my physics lobe?
🤖 You stored a comprehensive explanation of quantum mechanics in your physics lobe. The content covers how quantum mechanics differs from classical physics, describing key concepts like quantized energy levels, wave-particle duality, and quantum superposition. The system generated a reflection showing recurring themes: quantum(3), mechanics(2), physics(2), indicating the conceptual focus of your stored knowledge.

You> {"action":"stats","args":{}}
🔧 Action: stats
✅ Success
   Total memories: 2
   Top lobes: [('chat', 1), ('physics', 1)]
```

---

## Key Features Demonstrated

### **Intelligent Memory**
- **TF-IDF summarization** for content >500 characters
- **Content-addressed storage** using BLAKE3 hashing
- **Lobe organization** for different knowledge domains

### **Reflection System**
- **Keyword frequency analysis** from stored summaries  
- **Deterministic themes** computed from recent memories
- **No hidden ML** - transparent, auditable reflection

### **Built-in Safety**
- **Ethics preprocessing** via TOML contracts
- **Audit logging** for all operations
- **Local-first** - your data stays on your machine

### **Tool Integration**
- **JSON action parsing** from LLM responses
- **Seamless chat/tool switching** based on response format
- **Error handling** with graceful fallbacks

---

## Design Principles

* **AI Symbiosis**: agents that collaborate with humans/tools, not replace them.
* **Local-first**: works offline; simple files + SQLite; easy to audit and ship.
* **Deterministic reflection**: no hidden heuristics; themes from summaries only.
* **Separation of concerns**: hot vs. cold storage; orchestration vs. storage; LLM vs. memory/ethics.
* **Idempotent init**: safe to call often; one writer to the DB.

---

## What's in the box

* `Memory` (SQLite) — hot cache for bytes + summaries + metadata
* `Archivist` (FS) — content-addressed blobs by BLAKE3 CID
* `Librarian` — ingest (ID gen, optional summarization, reflection seed)
* `Commands` — `remember`, `reflect`, `stats`
* `Audit/Logbook` — JSONL streams seeded at init
* `Ethos` (precheck/decision gate) — rules seeded from TOML


## Deployment Options

### 🖥️ **Desktop Application** (Recommended)
Package as a standalone executable that users can download and run locally:

```bash
pip install pyinstaller
pyinstaller --onefile --windowed demo/app.py
```


## Roadmap

* Configurable reflection (swap frequency analysis for TF-IDF/embeddings)
* Richer consent & redaction flows in Ethos
* More agent adapters (tools/functions)
* Desktop GUI application
* Benchmarks and stress tests for multi-agent scenarios


## License

Licensed under **Apache License 2.0** — see [LICENSE](./LICENSE).

> Free to use, fork, and build upon. Preserve the author and ethical mission.


## Author

**Janay Harris**
AI Architect · Cloud Dev · Ethics Researcher
[LinkedIn](https://www.linkedin.com/in/janay-codes/) · [janayharris@synaptik-core.dev](mailto:janlynnh.916@gmail.com)


## Citation

> Harris, J. (2025). *Synaptik-Core: Toward Trustworthy AGI via Hybrid Cognitive Architecture*. ColorStack Summit 2025.


## Vision

> Synaptik Core is built on the belief that intelligence without memory is reactive, and intelligence without ethics is dangerous. It provides both.