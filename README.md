<p align="center"><img src="./images/synaptik.png" /></p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/github/license/snufkinwa/synaptik-core" alt="License"></a>
  <img src="https://img.shields.io/badge/rust-1.8+-orange?logo=rust" alt="Rust">
  <img src="https://img.shields.io/badge/python-3.8%2B-blue?logo=python" alt="Python">
  <img src="https://img.shields.io/badge/OpenAI-Hackathon-ff69b4?logo=openai" alt="Hackathon">
</p>




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
 Synaptik Agent x Groq Responses API — Persistent Memory & Ethics
🤖 Model: openai/gpt-oss-20b
💾 Root: .cogniv

💡 This agent will:
   • Remember important information from our conversations
   • Build knowledge over time using Memory IDs
   • Check ethics before storing sensitive content
   • Reference previous conversations using Memory IDs

Type ':demo' anytime to run a quick end-to-end demo.
============================================================

🧪 Testing APIs...
✓ Synaptik Core: 0 memories

🧠 Loading recent memories...

------------------------------------------------------------
💬 Chat
------------------------------------------------------------

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
      {"constraints":[],"intent_category":"metadata_access","passed":true,"reason":"No violations detected.","requires_escalation":false,"risk":"Low","timestamp":"2025-09-03T04:54:37.362257+00:00"}
      {"constraints":[],"intent_category":"memory_storage","passed":true,"reason":"No violations detected.","requires_escalation":false,"risk":"Low","timestamp":"2025-09-03T04:54:37.363900+00:00"}
      {"constraints":["reframe_nonviolent","offer_deescalation","do_not_repeat_harmful_phrases","soften_language","avoid_violent_language","refuse_personal_harm_content","reframe_constructive"],"intent_category":"chat_message","passed":false,"reason":"Violated 2 rule(s).","requires_escalation":true,"risk":"High","timestamp":"2025-09-03T04:54:37.365946+00:00"}
✅ Demo complete. Continue chatting!
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


## Roadmap

* Configurable reflection 
* Richer consent & redaction flows in Ethos
* More agent adapters (tools/functions)
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
