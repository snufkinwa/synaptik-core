from typing import Optional, List, Tuple, Dict, Any
from synaptik_core import PyCommands


class MemoryBridge:
    """Ergonomic wrapper over PyCommands.

    - Provides small helpers for recent + recall flows
    - Surfaces unified recall() returning {content, source}
    - Keeps printing out recall sources in helpers where appropriate
    """

    def __init__(self) -> None:
        self.cmd = PyCommands()

    # -------- Basic ops --------
    def root(self) -> str:
        return self.cmd.root()

    def stats(self, lobe: Optional[str] = None) -> Dict[str, Any]:
        return self.cmd.stats(lobe)

    def remember(self, lobe: str, content: str, key: Optional[str] = None) -> str:
        return self.cmd.remember(lobe, content, key)

    def reflect(self, lobe: str, window: int) -> str:
        return self.cmd.reflect(lobe, window)

    def recent(self, lobe: str, n: int = 10) -> List[str]:
        return self.cmd.recent(lobe, n)

    # -------- Unified recall --------
    def recall(self, memory_id: str, prefer: Optional[str] = None) -> Optional[Dict[str, Any]]:
        """Return {content, source} or None."""
        try:
            r = self.cmd.recall(memory_id, prefer)
            if isinstance(r, dict):
                return r
            return None
        except Exception:
            return None

    def get(self, memory_id: str, prefer: Optional[str] = None) -> Optional[str]:
        """Return just content (str) or None, with optional tier preference."""
        try:
            return self.cmd.recall_prefer(memory_id, prefer)
        except Exception:
            return None

    def recall_many(self, memory_ids: List[str], prefer: Optional[str] = None) -> List[Dict[str, Any]]:
        out: List[Dict[str, Any]] = []
        if not memory_ids:
            return out
        try:
            arr = self.cmd.recall_many(memory_ids, prefer)
            if isinstance(arr, list):
                return list(arr)
        except Exception:
            pass
        return out

    # -------- Convenience helpers for demos --------
    def recent_with_content(self, lobe: str, n: int = 3, prefer: Optional[str] = None) -> List[Tuple[str, Optional[Dict[str, Any]]]]:
        ids = self.recent(lobe, n) or []
        pairs: List[Tuple[str, Optional[Dict[str, Any]]]] = []
        for mid in ids:
            pairs.append((mid, self.recall(mid, prefer)))
        return pairs

    def print_recall_preview(self, memory_id: str, prefer: Optional[str] = None, width: int = 80) -> None:
        r = self.recall(memory_id, prefer)
        if not r or not r.get("content"):
            print(f"   ❌ {memory_id[:18]}... not found")
            return
        src = r.get("source", "auto")
        text = (r.get("content") or "")
        prev = text[:width] + ("..." if len(text) > width else "")
        print(f"   🔎 {memory_id[:18]}... source={src} content='{prev}'")
