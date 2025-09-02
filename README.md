<p align="center"><img src="./images/synaptik.png" /></p>

# Synaptik Core

**AI Symbiosis, not just automation.**
A lightweight cognitive kernel for agents: local memory, simple reflection, and auditable guardrails — designed to work *with* people and tools.


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

---

## Complete Example with Groq Responses API

Here's a working demo that shows Synaptik Core in action with Groq's OpenAI models:

```python
# demo/MVP_groq_responses.py
import os, json, re, time
from pathlib import Path
from dotenv import load_dotenv
from openai import OpenAI
from synaptik_core import PyCommands

# Load environment variables
load_dotenv(Path(__file__).resolve().parents[1] / ".env")

# Groq Responses API setup
MODEL = os.environ.get("GROQ_MODEL", "openai/gpt-oss-20b")
client = OpenAI(
    api_key=os.environ.get("GROQ_API_KEY"),
    base_url="https://api.groq.com/openai/v1",
)

# Initialize Synaptik Core
cmd = PyCommands()

SYSTEM = """You are the Synaptik Agent.
- Synaptik Core handles persistence/reflection/stats. You are stateless.
- When you need an action, emit ONE JSON object line:
  {"action":"remember","args":{"lobe":"chat","content":"...","key":null}}
  {"action":"reflect","args":{"lobe":"chat","window":50}}
  {"action":"stats","args":{"lobe":null}}
- Otherwise, reply in plain text.
"""

def maybe_parse_action(text: str):
    """Parse JSON action from LLM response."""
    t = text.strip()
    if t.startswith("{") and t.endswith("}"):
        try:
            return json.loads(t)
        except Exception:
            pass
    m = re.search(r"\{.*?\}", text, flags=re.DOTALL)
    if m:
        try:
            return json.loads(m.group(0))
        except Exception:
            return None
    return None

def tool_router(action):
    """Execute Synaptik Core actions."""
    name = (action or {}).get("action")
    args = (action or {}).get("args", {}) or {}
    
    try:
        if name == "remember":
            return {"ok": True, "memory_id": cmd.remember(
                args.get("lobe", "notes"),
                args.get("content", ""),
                args.get("key"),
            )}
        elif name == "reflect":
            return {"ok": True, "reflection": cmd.reflect(
                args.get("lobe", "notes"),
                int(args.get("window", 20)),
            )}
        elif name == "stats":
            return {"ok": True, "stats": cmd.stats(args.get("lobe"))}
        else:
            return {"ok": False, "error": f"unknown action: {name}"}
    except Exception as e:
        return {"ok": False, "error": str(e)}

def chat(messages, retries=2):
    """Call Groq Responses API with retry logic."""
    # Convert messages to input string for Responses API
    input_text = ""
    for msg in messages:
        if msg["role"] == "system":
            input_text += f"{msg['content']}\n\n"
        elif msg["role"] == "user":
            input_text += f"User: {msg['content']}\n\n"
        elif msg["role"] == "assistant":
            input_text += f"Assistant: {msg['content']}\n\n"
    
    for attempt in range(retries + 1):
        try:
            response = client.responses.create(
                model=MODEL,
                input=input_text.strip(),
                temperature=0.2,
                max_output_tokens=512,
                reasoning={"effort": "medium"}
            )
            return response.output_text or ""
        except Exception as e:
            if attempt < retries and "500" in str(e):
                time.sleep(0.6 * (attempt + 1))
                continue
            # Fallback to regular chat API
            try:
                response = client.chat.completions.create(
                    model=MODEL,
                    messages=messages,
                    temperature=0.2,
                    max_tokens=512,
                )
                return response.choices[0].message.content or ""
            except Exception:
                raise e

def run_demo():
    """Interactive demo of Synaptik Core + Groq."""
    print("🧠 Synaptik Core + Groq Demo")
    print(f"📁 Data directory: {cmd.root()}")
    print(f"🤖 Model: {MODEL}")
    print("\nType 'exit' to quit\n")

    convo = [{"role": "system", "content": SYSTEM}]
    
    while True:
        try:
            user_input = input("You> ").strip()
        except (EOFError, KeyboardInterrupt):
            print("\nGoodbye! 👋")
            break
            
        if user_input.lower() in {"exit", "quit", "q"}:
            break
        
        if not user_input:
            continue

        convo.append({"role": "user", "content": user_input})
        
        try:
            assistant_response = chat(convo)
        except Exception as e:
            print(f"❌ Error: {e}")
            convo.pop()
            continue

        # Check if response contains a tool action
        action = maybe_parse_action(assistant_response)
        if action:
            # Execute the action
            try:
                result = tool_router(action)
                print(f"🔧 Action: {action.get('action', 'unknown')}")
                
                if result.get("ok"):
                    print("✅ Success")
                    if "memory_id" in result:
                        print(f"   Memory ID: {result['memory_id'][:20]}...")
                    if "reflection" in result and result["reflection"]:
                        print(f"   Reflection: {result['reflection']}")
                    if "stats" in result:
                        stats = result["stats"]
                        print(f"   Total memories: {stats.get('total', 0)}")
                        if stats.get('by_lobe'):
                            top_lobes = stats['by_lobe'][:3]
                            print(f"   Top lobes: {top_lobes}")
                else:
                    print(f"❌ Failed: {result.get('error', 'Unknown error')}")
                
                # Add tool result to conversation for context
                convo.append({"role": "assistant", "content": assistant_response})
                convo.append({"role": "user", "content": f"[Tool result: {json.dumps(result)}]"})
                
            except Exception as e:
                print(f"❌ Tool error: {e}")
                convo.append({"role": "assistant", "content": assistant_response})
        else:
            # Regular chat response
            print(f"🤖 {assistant_response}")
            convo.append({"role": "assistant", "content": assistant_response})

        # Keep conversation manageable
        if len(convo) > 20:
            convo = [convo[0]] + convo[-18:]

if __name__ == "__main__":
    run_demo()
```

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
- **Automatic summarization** for content >500 characters
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
* `LobeStore` — simple versioned object store per lobe (for blobs)
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
[LinkedIn](https://www.linkedin.com/in/janay-codes/) · [janayharris@synaptik-core.dev](mailto:janayharris@synaptik-core.dev)


## Citation

> Harris, J. (2025). *Synaptik-Core: Toward Trustworthy AGI via Hybrid Cognitive Architecture*. ColorStack Summit 2025.


## Vision

> Intelligence without memory is reactive.
> Intelligence without ethics is dangerous.
> **Synaptik Core is the foundation for both.**