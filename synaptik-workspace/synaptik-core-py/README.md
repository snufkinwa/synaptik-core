<p align="center"><img src="https://res.cloudinary.com/dindjf2vu/image/upload/v1757209651/synaptik_vt1cpy.png"/></p>

# Synaptik Core (Python)

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/github/license/snufkinwa/synaptik-core" alt="License"></a>
  <a href="https://pypi.org/project/synaptik-core-beta/"><img src="https://img.shields.io/pypi/v/synaptik-core-beta.svg" alt="PyPI version"></a>
  <img src="https://img.shields.io/badge/OpenAI-Hackathon-ff69b4?logo=openai" alt="Hackathon">
</p>


Python bindings for Synaptik Core — memory, ethics, and audit for trustworthy agents.

## Install

```bash
pip install synaptik-core-beta
```

## Quick Start

```python
import synaptik_core, json

cmd = synaptik_core.PyCommands()

# Write + read
mid = cmd.remember("chat", "hello")
print(cmd.recall(mid))                 # -> {id, content, source}

# Prefer a tier explicitly
print(cmd.recall(mid, prefer="hot"))  # -> {id, content, source="hot"}

# Recent IDs (newest -> oldest)
print(cmd.recent("chat", 5))

# Ethics precheck
print(cmd.precheck_text("hi", purpose="message_generation"))

# Replay (immutable snapshots) — no hashes required
path = cmd.begin_branch("chat", "alt")      # resolves base automatically
new_hash = cmd.extend_path(path, json.dumps({"t": 1}))
print(cmd.recall_latest_on_path(path))        # -> {content, meta}

# Provenance (citations)
print(cmd.cite_sources(new_hash))

```

## Branching and Consolidation (FF‑only)

High-level helpers move seeding, normalization, and provenance into the core.

```python
import synaptik_core as sc
cmd = sc.PyCommands()

# Sprout a dendrite; base resolution:
# - base=None,lobe=None: start from head('cortex') if present, else seed from 'chat'
# - base provided: interpreted as an existing path name (if it exists), otherwise as a CID
base_cid = cmd.sprout_dendrite(path="feature-x", base=None, lobe=None)

# Encode an engram (Ethos-gated) with optional meta
cid = cmd.encode_engram(path="feature-x", content="...", meta={"file": "lru.rs"})

# Systems consolidation to cortex (FF-only; errors if not FF)
head = cmd.systems_consolidate(src_path="feature-x", dst_path="cortex")

# Inspect history and recall
trace = cmd.trace_path("feature-x", limit=10)
snap = cmd.recall(head)           # dict or None
content = cmd.recall_prefer(head) # str or None
```

Notes:
- Paths normalize to lowercase; “feature-x” and “feature_x” are equivalent.
- Consolidation is fast‑forward only; binds are not yet supported.
- Prefer `sprout_dendrite()/encode_engram()/systems_consolidate()`; legacy `begin_branch/extend_path` remain for compatibility. Aliases `branch/append/consolidate` are available.

## License

Apache-2.0 — see `LICENSE`.
