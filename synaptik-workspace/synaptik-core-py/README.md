<p align="center"><img src="https://res.cloudinary.com/dindjf2vu/image/upload/v1757209651/synaptik_vt1cpy.png"/></p>

# Synaptik Core (Python bindings)

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/github/license/snufkinwa/synaptik-core" alt="License"></a>
<a href="https://pypi.org/project/synaptik-core-beta/">
  <img src="https://img.shields.io/pypi/v/synaptik-core-beta.svg" alt="PyPI version">
</a>
  <img src="https://img.shields.io/badge/OpenAI-Hackathon-ff69b4?logo=openai" alt="Hackathon">
</p>

**Synaptik Core** is the memory, ethics, and audit substrate for trustworthy AI agents.  
This package provides Python bindings for the Rust core engine.


##  Installation

Install from PyPI (prebuilt wheels available):

```bash
pip install synaptik-core-beta
```

- Prebuilt wheels: macOS x86_64 (11+), Linux x86_64 (manylinux2014)
- Other platforms/arches: build from source (see below)
- Requires Python 3.8+


## ⚡ Quick Start

Install the package, then use the `PyCommands` class from the `synaptik_core` module.

```bash
pip install synaptik-core-beta
```

```python
import synaptik_core

# Initialize command surface
cmd = synaptik_core.PyCommands()

# Write a memory (lobe = logical namespace)
memory_id = cmd.remember("chat", "hello from alice")

# Read it back (returns dict with id/content/source)
print(cmd.recall(memory_id))

# Recent items within a lobe
print(cmd.recent("chat", n=5))

# Pre-check text against ethos contracts
print(cmd.precheck_text("generate a friendly reply", purpose="message_generation"))
```

##  Features

*  **Contract-based safeguards** — run WASM contracts to enforce rules
*  **Persistent memory** — short-term (SQLite hot cache) + long-term archival
* **Transparent audit log** — append-only JSONL for all decisions
*  **Resource limits** — bounded execution for safe, deterministic behavior

## 🛠 Development

This package is built with [Maturin](https://github.com/PyO3/maturin) and [PyO3](https://pyo3.rs/).
Rust ≥ 1.70 and Python ≥ 3.8 required for local builds.

From source (for unsupported platforms/arches):

```bash
# in this directory
pip install maturin
maturin develop --release
```


## 📄 License

Licensed under the [Apache 2.0 License](https://www.apache.org/licenses/LICENSE-2.0).
