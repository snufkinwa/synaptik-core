from typing import Any, Dict, Optional

try:
    from .memory_bridge import MemoryBridge
    from .path_utils import safe_trace_path
except ImportError:  # runtime when run as a module
    from memory_bridge import MemoryBridge
    from path_utils import safe_trace_path


def maybe_parse_action(text: str) -> Optional[Dict[str, Any]]:
    import json, re
    lines = (text or "").strip().split('\n')
    if lines:
        last = lines[-1].strip()
        if last.startswith('{') and last.endswith('}'):
            try:
                return json.loads(last)
            except Exception:
                pass
    m = re.search(r'\{[^{}]*"action"[^{}]*\}', text or "", flags=re.DOTALL)
    if m:
        try:
            return json.loads(m.group(0))
        except Exception:
            return None
    return None


def route(action: Dict[str, Any], bridge: MemoryBridge) -> Dict[str, Any]:
    name = (action or {}).get("action")
    args = (action or {}).get("args", {}) or {}

    if name == "remember":
        mid = bridge.remember(args.get("lobe", "chat"), args.get("content", ""), args.get("key"))
        return {"ok": True, "memory_id": mid}

    if name == "reflect":
        note = bridge.reflect(args.get("lobe", "chat"), int(args.get("window", 20)))
        return {"ok": True, "reflection": note}

    if name == "stats":
        return {"ok": True, "stats": bridge.stats(args.get("lobe"))}

    if name == "root":
        try:
            return {"ok": True, "root": bridge.root()}
        except Exception as e:
            return {"ok": False, "error": str(e)}

    if name == "verify_persistence":
        import os
        try:
            root = bridge.root()
            cache_db = os.path.join(root, "cache", "memory.db")
            dag_nodes = os.path.join(root, "dag", "nodes")
            archive_dir = os.path.join(root, "archive")
            logbook = os.path.join(root, "logbook")
            res = {
                "root": root,
                "db_exists": os.path.exists(cache_db),
                "dag_nodes_exists": os.path.isdir(dag_nodes),
                "archive_exists": os.path.isdir(archive_dir),
                "logbook_exists": os.path.isdir(logbook),
            }
            return {"ok": True, "persistence": res}
        except Exception as e:
            return {"ok": False, "error": str(e)}

    if name == "precheck":
        # direct passthrough to PyCommands for now
        res = bridge.cmd.precheck_text(args.get("text", ""), args.get("purpose", "general"))
        return {"ok": True, "precheck_result": res}

    if name == "recent":
        ids = bridge.recent(args.get("lobe", "chat"), int(args.get("n", 10)))
        return {"ok": True, "recent_ids": ids}

    if name == "recall":
        mem_id = args.get("memory_id", "")
        if not mem_id:
            return {"ok": False, "error": "memory_id required"}
        prefer = args.get("prefer")
        r = bridge.recall(mem_id, prefer)
        return {"ok": True, "recall": r}

    if name == "recall_many":
        ids = args.get("memory_ids") or []
        if not isinstance(ids, list) or not ids:
            return {"ok": False, "error": "memory_ids (list) required"}
        prefer = args.get("prefer")
        arr = bridge.recall_many(list(map(str, ids)), prefer)
        return {"ok": True, "results": arr}

    if name == "recall_sources":
        mem_id = args.get("memory_id", "")
        if not mem_id:
            return {"ok": False, "error": "memory_id required"}
        order = args.get("order") or ["hot", "archive", "dag"]
        out: list[dict] = []
        for prefer in order:
            r = bridge.recall(mem_id, prefer)
            out.append({
                "prefer": prefer,
                "found": bool(r and r.get("content")),
                "result": r or None,
            })
        return {"ok": True, "id": mem_id, "sources": out}

    if name == "cite_sources":
        """Cite provenance sources for a DAG snapshot.

        Args (one of):
          - snapshot_id: blake3 hash id of the DAG node
          - path_name: use newest snapshot on this path (head)
          - memory_id: resolve to DAG node via refs/ids index
        """
        import json, os, re

        def _sanitize(s: str) -> str:
            return "".join(c if c.isascii() and c.isalnum() else "_" for c in (s or ""))

        snapshot_id = args.get("snapshot_id")
        path_name = args.get("path_name")
        memory_id = args.get("memory_id")

        hash_id = None
        try:
            if isinstance(snapshot_id, str) and snapshot_id:
                hash_id = snapshot_id
            elif isinstance(path_name, str) and path_name:
                h = bridge.cmd.latest_on_path(path_name)
                if isinstance(h, str):
                    hash_id = h
            elif isinstance(memory_id, str) and memory_id:
                # Resolve via refs/ids index
                root = bridge.root()
                idx = os.path.join(root, "refs", "ids", f"{_sanitize(memory_id)}.json")
                try:
                    with open(idx, "r", encoding="utf-8") as f:
                        v = json.load(f)
                    node_name = v.get("node")
                except Exception:
                    node_name = None
                if node_name:
                    node_path = os.path.join(root, "dag", "nodes", node_name)
                    try:
                        with open(node_path, "r", encoding="utf-8") as f:
                            nv = json.load(f)
                        h = nv.get("hash")
                        if isinstance(h, str) and h:
                            hash_id = h
                    except Exception:
                        pass
        except Exception:
            hash_id = None

        if not hash_id:
            return {"ok": False, "error": "could not resolve snapshot hash from args"}

        try:
            cites = bridge.cmd.cite_sources(hash_id)
            return {"ok": True, "snapshot_id": hash_id, "provenance": cites}
        except Exception as e:
            return {"ok": False, "error": str(e)}

    if name == "trace_path":
        path = args.get("path_name")
        if not path:
            return {"ok": False, "error": "path_name required"}
        limit = int(args.get("limit", 10))
        try:
            v = bridge.cmd.trace_path(path, limit)
            if isinstance(v, list) and len(v) > 1:
                return {"ok": True, "path": path, "items": v}
        except Exception:
            pass
        # Fallback to Python-implemented trace to handle older nodes
        items = safe_trace_path(bridge.cmd, bridge.root(), path, limit)
        return {"ok": True, "path": path, "items": items}

    if name == "recall_latest_on_path":
        path = args.get("path_name")
        if not path:
            return {"ok": False, "error": "path_name required"}
        try:
            v = bridge.cmd.recall_latest_on_path(path)
            return {"ok": True, "path": path, "snapshot": v}
        except Exception as e:
            return {"ok": False, "error": str(e)}

    return {"ok": False, "error": f"unknown action: {name}"}
    if name == "branch_hop":
        import json, time
        cmd = bridge.cmd
        # Parse args with sensible defaults
        lobe = str(args.get("lobe", "chat"))
        branch_a = args.get("branch_a") or args.get("a") or "plan-a"
        branch_b = args.get("branch_b") or args.get("b")
        a_steps = int(args.get("a_steps", 2))
        b_steps = int(args.get("b_steps", 1))

        # Choose a base: prefer lobe base, else last recalled, else error
        try:
            base = cmd.seed_base_from_lobe(lobe) or cmd.last_recalled_id()
        except Exception:
            base = None
        if not base:
            return {"ok": False, "error": f"no base snapshot available for lobe '{lobe}'"}
        try:
            cmd.recall_snapshot(base)
        except Exception:
            pass

        def _append(path: str, payload: dict) -> str:
            meta = {
                "kind": "branch_step",
                "branch": path,
                "provenance": {"sources": [{"kind": "dag", "uri": f"dag:{base}", "cid": base}]},
            }
            return cmd.extend_path(path, json.dumps(payload), meta)

        # Branch A
        try:
            a_id = cmd.recall_and_diverge(base, branch_a)
        except Exception:
            a_id = cmd.diverge_from(base, branch_a)
        a_cids: list[str] = []
        for i in range(1, a_steps + 1):
            a_cids.append(_append(a_id, {"step": str(i), "note": f"step {i} on {branch_a}"}))

        # Branch B (optional)
        b_id = None
        b_cids: list[str] = []
        if branch_b:
            try:
                cmd.recall_snapshot(base)
            except Exception:
                pass
            try:
                b_id = cmd.recall_and_diverge(base, branch_b)
            except Exception:
                b_id = cmd.diverge_from(base, branch_b)
            for i in range(1, b_steps + 1):
                b_cids.append(_append(b_id, {"step": str(i), "note": f"step {i} on {branch_b}"}))

        # Brief settle
        time.sleep(0.05)

        res = {
            "ok": True,
            "base": base,
            "branch_a": {"id": a_id, "steps": a_steps, "cids": a_cids},
        }
        if b_id:
            res["branch_b"] = {"id": b_id, "steps": b_steps, "cids": b_cids}
        return res
