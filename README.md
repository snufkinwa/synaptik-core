![Synaptik Logo](./images/synaptik.png)

# Synaptik Core

**AI Symbiosis, not just automation.**
A lightweight cognitive kernel for agents: local memory, simple reflection, and auditable guardrails — designed to work *with* people and tools.

---

## What is `synaptik-core`?

A small Rust library (with Python bindings) that gives LLM agents durable memory and simple, deterministic reflection:

* **Local-first workspace** — `.cogniv/` with `cache/memory.db` (SQLite) + `archive/` (content-addressed files).
* **Single-writer Memory** — one SQLite handle, safe & idempotent init.
* **Archivist** — file-only cold storage by BLAKE3 CID (no DB writes).
* **Librarian** — orchestrates ingest: optional summarization for long inputs, plus a tiny keyword reflection seed.
* **Commands** — high-level API: `remember`, `reflect`, `stats`.
* **Audit / Ethos hooks** — actions are recorded to JSONL logbook; an ethos precheck gate is in place (TOML contract seeded).

This gives agents structure and recall without cloud lock-in, and keeps behavior auditable for safety and trust.

---

## Architecture (MVP)

```
User / Agent
   │
   ├─► Commands  ──► Librarian ──► Memory (SQLite: .cogniv/cache/memory.db)
   │                      │
   │                      └─► Archivist (FS: .cogniv/archive/<cid>)
   │
   └─► Ethos/Audit hooks (JSONL logbook in .cogniv/)
```

* **Summaries**: created only for long inputs (current threshold: \~500 chars, via the `summary` crate).
* **Reflection**: periodic “themes” line computed from *recent summaries* (keyword frequency; deterministic, offline).
* **Init**: the first call creates `.cogniv/` and seeds config, contracts, and logbook (idempotent).

---

## Install (Python)

Requirements: Python ≥ 3.8, Rust toolchain, `maturin`.

```bash
# build the Python extension in editable mode
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

> Tip: to see a non-empty `reflect`, ingest 3–4 longer notes (>500 chars) that share a few repeated terms, then call `reflect("chat", window)`.

---

## Use with Groq or Ollama

Let the LLM plan, but keep memory/ethics local. The model emits a tiny **action JSON**; your host calls `PyCommands`.

```python
from groq import Groq
from synaptik_core import PyCommands
import json, re, os

os.environ.setdefault("GROQ_API_KEY", "<your-key>")
client = Groq()
cmd = PyCommands()

SYSTEM = """You are the Synaptik Agent.
- Synaptik Core handles persistence/reflection/stats. You are stateless.
- Available actions (emit ONE JSON line when needed):
  {"action":"remember","args":{"lobe":"chat","content":"...","key":null}}
  {"action":"reflect","args":{"lobe":"chat","window":50}}
  {"action":"stats","args":{"lobe":null}}
- Otherwise answer in plain text.
"""

def maybe_parse_action(text):
    t = text.strip()
    if t.startswith("{") and t.endswith("}"):
        return json.loads(t)
    m = re.search(r"\{.*?\}", text, flags=re.DOTALL)
    return json.loads(m.group(0)) if m else None

def call(action):
    a, x = action["action"], action.get("args", {})
    if a == "remember":
        return {"ok": True, "memory_id": cmd.remember(x.get("lobe","notes"), x.get("content",""), x.get("key"))}
    if a == "reflect":
        return {"ok": True, "reflection": cmd.reflect(x.get("lobe","notes"), int(x.get("window",20)))}
    if a == "stats":
        return {"ok": True, "stats": cmd.stats(x.get("lobe"))}
    return {"ok": False, "error": f"unknown action {a}"}

msgs = [{"role":"system","content":SYSTEM},
        {"role":"user","content":"Please remember: I like concise Rust tips."}]
out = client.chat.completions.create(model="openai/gpt-oss-20b", messages=msgs, temperature=0.2)
txt = out.choices[0].message.content
act = maybe_parse_action(txt)
print(call(act) if act else txt)
```

---

## Design Principles

* **AI Symbiosis**: agents that collaborate with humans/tools, not replace them.
* **Local-first**: works offline; simple files + SQLite; easy to audit and ship.
* **Deterministic reflection**: no hidden heuristics; tags from summaries only.
* **Separation of concerns**: hot vs. cold storage; orchestration vs. storage; LLM vs. memory/ethics.
* **Idempotent init**: safe to call often; one writer to the DB.

---

## What’s in the box (today)

* `Memory` (SQLite) — hot cache for bytes + summaries + metadata
* `Archivist` (FS) — content-addressed blobs by BLAKE3 CID
* `Librarian` — ingest (ID gen, optional summarization, reflection seed)
* `Commands` — `remember`, `reflect`, `stats`
* `LobeStore` — simple versioned object store per lobe (for blobs)
* `Audit/Logbook` — JSONL streams seeded at init
* `Ethos` (precheck/decision gate) — rules seeded from TOML

---

## Roadmap

* Configurable reflection (swap freq for TF-IDF/embeddings when desired)
* Richer consent & redaction flows in Ethos
* More agent adapters (tools/functions)
* Benchmarks and stress tests for multi-agent scenarios

---

## License

Licensed under **Apache License 2.0** — see [LICENSE](./LICENSE).

> Free to use, fork, and build upon. Preserve the author and ethical mission.

---

## Author

**Janay Harris**
AI Architect · Cloud Dev · Ethics Researcher
[LinkedIn](https://www.linkedin.com/in/janay-codes/) · [janayharris@synaptik-core.dev](mailto:janayharris@synaptik-core.dev)

---

## Citation

> Harris, J. (2025). *Synaptik-Core: Toward Trustworthy AGI via Hybrid Cognitive Architecture*. ColorStack Summit 2025.

---

## Vision

> Intelligence without memory is reactive.
> Intelligence without ethics is dangerous.
> **Synaptik Core is the foundation for both.**
