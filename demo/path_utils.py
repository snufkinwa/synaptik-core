from __future__ import annotations

import json
import os
from typing import List, Dict, Optional


def _sanitize(name: str) -> str:
    return "".join(c if c.isascii() and c.isalnum() else "_" for c in name or "")


def _read_json(p: str) -> Optional[dict]:
    try:
        with open(p, "r", encoding="utf-8") as f:
            return json.load(f)
    except Exception:
        return None


def python_trace_path(root: str, path_name: str, limit: int = 50) -> List[Dict[str, str]]:
    """Trace newest->oldest by walking refs/paths and dag/nodes directly.

    Works around older engines where node.parent may be a content hash instead of a filename.
    Returns a list of dicts similar to cmd.trace_path: {hash, id, ts, lobe, key}.
    """
    out: List[Dict[str, str]] = []
    try:
        base = os.path.abspath(root or ".cogniv")
        paths_ref = os.path.join(base, "refs", "paths", f"{_sanitize(path_name)}.json")
        pr = _read_json(paths_ref)
        if not isinstance(pr, dict) or not pr.get("head_node"):
            return out
        cur = pr["head_node"]
        n = 0
        while isinstance(cur, str) and cur and n < int(limit):
            node_path = os.path.join(base, "dag", "nodes", cur)
            node = _read_json(node_path)
            if not isinstance(node, dict):
                break
            out.append(
                {
                    "filename": cur,
                    "id": str(node.get("id", "")),
                    "hash": str(node.get("hash", "")),
                    "ts": str(node.get("ts", "")),
                    "lobe": str(node.get("lobe", "")),
                    "key": str(node.get("key", "")),
                }
            )
            parent = node.get("parent")
            if isinstance(parent, str) and parent and parent.endswith(".json"):
                cur = parent
            elif isinstance(parent, str) and parent:
                # Resolve parent from content hash -> node filename via refs/hashes
                href = os.path.join(base, "refs", "hashes", f"{_sanitize(parent)}.json")
                hidx = _read_json(href) or {}
                next_name = str(hidx.get("node", ""))
                if next_name:
                    cur = next_name
                else:
                    break
            else:
                break
            n += 1
    except Exception:
        return []
    return out


def safe_trace_path(cmd, root: str, path_name: str, limit: int = 50):
    """Try engine trace_path, then fall back to python_trace_path if needed."""
    try:
        items = cmd.trace_path(path_name, limit)
        if isinstance(items, list) and len(items) > 1:
            return items
    except Exception:
        pass
    return python_trace_path(root, path_name, limit)


def print_citations(cmd, snap_id: str, label: str) -> None:
    """Pretty-print provenance as compact tokens per source on one line.

    Example:  🔗 Sources for plan_a @ 7408ed45: dag:33d825b8fd5f…, memory:preferences_8eeb…
    """
    try:
        cites = cmd.cite_sources(snap_id) or []
    except Exception:
        cites = []
    short = (snap_id or "")[:8]
    if not cites:
        print(f"   🔗 Sources for {label} @ {short}: []")
        return
    rows = []
    for c in cites:
        kind = c.get("kind", "?") if isinstance(c, dict) else "?"
        cid = ""
        if isinstance(c, dict):
            cid = c.get("cid") or c.get("uri") or ""
        token = f"{kind}:{(cid or '')[:12]}…"
        rows.append(token)
    print(f"   🔗 Sources for {label} @ {short}: " + ", ".join(rows))
