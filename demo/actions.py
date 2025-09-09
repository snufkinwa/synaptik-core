from typing import Any, Dict, Optional

try:
    from .memory_bridge import MemoryBridge
except ImportError:  # runtime when run as a module
    from memory_bridge import MemoryBridge


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

    return {"ok": False, "error": f"unknown action: {name}"}
