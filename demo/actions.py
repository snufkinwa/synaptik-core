from typing import Any, Dict

try:
    from .memory_bridge import MemoryBridge
except ImportError:
    from memory_bridge import MemoryBridge


def maybe_parse_action(text: str) -> Dict[str, Any] | None:
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

    return {"ok": False, "error": f"unknown action: {name}"}
