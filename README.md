

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

Lightweight Rust/Python core library that gives LLM agents durable memory and auditable ethics, stored locally in `.cogniv/`.

## Features

- **Local-first workspace** — SQLite cache + content-addressed file storage
- **Persistent memory** with automatic summarization and reflection  
- **Rewind & Diverge** — revisit any past memory, branch into a new thought stream, and preserve the full history of the mind  
- **Built-in ethics** via TOML contracts and audit trails
- **Python API (MemoryBridge)** — `root()`, `stats()`, `remember()`, `reflect()`, `recent()`, `recall()`, `get()`, `recall_many()`, plus helpers `recent_with_content()`, `print_recall_preview()`
- **Python Bindings (PyCommands)** — functions used in the demo (no Rust internals):
  - Ethics: `precheck_text()`
  - Replay: `seed_base_from_lobe()`, `last_recalled_id()`, `recall_snapshot()`, `recall_and_diverge()`, `extend_path()`, `trace_path()`, `recall_latest_on_path()`, `cite_sources()`
  - Neuroscience ops: `sprout_dendrite()`, `encode_engram()`, `systems_consolidate()`, `merge()`
  - Path helpers: `dag_head()`, `update_path_head()`
  - Misc: `root()`
- **No cloud dependency** — everything runs locally


## Installation

Set up an isolated Python environment (conda or venv) and install dependencies.

Option A — conda (recommended on Apple Silicon):

```bash
conda create -n synaptik python=3.9 -y
conda activate synaptik
python -m pip install --upgrade pip
pip install -r demo/requirements.txt
```

Option B — venv (macOS/Linux):

```bash
python -m venv .venv
source .venv/bin/activate
python -m pip install --upgrade pip
pip install -r demo/requirements.txt
```

View core package on [PyPI↗](https://pypi.org/project/synaptik-core-beta/)

- macOS Apple Silicon arm64; Linux x86_64 (manylinux2014)
- Windows: no wheel currently; use WSL2 or build [From Source](#from-source-unsupported-platforms-eg-windows)
- Requires Python 3.8+

### From Source (unsupported platforms, e.g., Windows)

If a prebuilt wheel is not available for your platform/architecture, build locally with maturin.

```bash
cd synaptik-workspace/synaptik-core-py
pip install maturin
maturin develop --release
```

## Quick Start

Demo entrypoint: `demo/demo.py` (Groq chat demo)

1) Ensure dependencies are installed (see Installation).

2) Add your Groq API key in a `.env` at the project root:

```
GROQ_API_KEY=your_api_key_here
GROQ_MODEL=openai/gpt-oss-20b
```

3) Run the chat demo:

```bash
python -m demo.demo
```

To reproduce the exact flow shown in the demo video, run `:demo`. Then open and paste the prompts from:

- `demo/test_prompts_syn.txt`

Paste them phase-by-phase (Phase 1 → Phase 16) into the REPL after running `python -m demo.demo`. This will exercise:

- Persistent notes and preferences in the `preferences` lobe
- Promotion into archive and DAG with recall previews and sources
- Ethics precheck decisions (allow/allow_with_constraints/block)
- Logging into `.cogniv/logbook/` with audit trails

## Why It Matters

LLMs are like toddlers: sponges for patterns, but unsafe without guidance. Just as a toddler needs a parent to cross the street, an LLM needs an outer layer that enforces safe, accountable behavior.  

Even in software, React has a **parent container** that wraps and organizes child components — without it, the system breaks down.  

**Synaptik Core is that layer for AI:** durable memory, enforceable ethics, and verifiable accountability.

## License

Apache License 2.0

---

*Intelligence without memory is reactive. Intelligence without ethics is dangerous. Synaptik Core provides both.*
