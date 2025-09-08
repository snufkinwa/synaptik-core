import re
from typing import List, Dict

from memory_bridge import MemoryBridge
from ui import print_assistant


def looks_like_recent_query(text: str) -> bool:
    t = (text or "").lower()
    return (("recent" in t or "latest" in t) and ("memory" in t or "memories" in t)) or (
        "show" in t and "memories" in t and "source" in t
    )


def handle_recent_query(mem: MemoryBridge, user: str, convo: List[Dict[str, str]]) -> None:
    lobes = ["preferences", "solutions", "insights", "chat", "notes"]
    t = user.lower()
    lobe = next((l for l in lobes if l in t), "chat")
    mnum = re.search(r"(last|recent)\s+(\d{1,3})", t)
    n = int(mnum.group(2)) if mnum else 10

    ids = mem.recent(lobe, n) or []
    print(f"\n📋 Recent memories (lobe={lobe}, n={len(ids)}):")
    details: list[str] = []
    if ids:
        # Batch recall for speed
        results = mem.recall_many(ids[:n], "auto")
        for i, (mid, r) in enumerate(zip(ids[:n], results), start=1):
            if isinstance(r, dict) and r.get("content"):
                txt = (r["content"] or "").strip().replace("\n", " ")
                if len(txt) > 100:
                    txt = txt[:100] + "..."
                src = r.get("source", "auto")
                print(f"  {i}. {mid[:20]}…  [src={src}]  {txt}")
                details.append(f"{mid}:{src}")
            else:
                print(f"  {i}. {mid[:20]}…  (not found)")
    if details:
        convo.append({"role": "user", "content": "[Recent memories retrieved: " + ", ".join(details) + "]"})


def looks_like_source_test(text: str) -> bool:
    t = (text or "").lower()
    return (("hot" in t and "archive" in t and "dag" in t) and ("memory" in t or "memories" in t or "profile" in t)) or (
        "test" in t and "sources" in t and ("memory" in t or "memories" in t)
    )


def handle_source_test(mem: MemoryBridge, user: str) -> None:
    # Choose a lobe to best demonstrate sources
    t = user.lower()
    lobes = ["preferences", "chat", "solutions", "insights", "notes"]
    # Map common nouns → lobe (e.g., "profile" → "preferences")
    chosen = None
    if "profile" in t:
        chosen = "preferences"
    else:
        chosen = next((l for l in lobes if l in t), None)
    if not chosen:
        try:
            st_chat = mem.stats("chat") or {}
            if int(st_chat.get("archived", 0)) > 0:
                chosen = "chat"
        except Exception:
            chosen = None
    if not chosen:
        chosen = "preferences"

    ids = mem.recent(chosen, 1) or []
    if not ids:
        print(f"\n📌 No memories found in '{chosen}'. Try saving one first or run :demo.")
        return

    mid = ids[0]
    print(f"\n🧪 Source test (lobe={chosen})")
    print(f"   ID: {mid}")
    for prefer in ("hot", "archive", "dag"):
        txt = mem.get(mid, prefer)
        if txt:
            t = (txt or "").strip().replace("\n", " ")
            if len(t) > 120:
                t = t[:120] + "..."
            print(f"  • {prefer.upper():7} → {t}")
        else:
            print(f"  • {prefer.upper():7} → (not found)")
    # Final summary line for easy copy/reference
    try:
        found = []
        for prefer in ("hot", "archive", "dag"):
            txt = mem.get(mid, prefer)
            if txt:
                found.append(prefer)
        print(f"   Summary: id={mid}, found={found if found else 'none'}")
    except Exception:
        pass


def looks_like_root_query(text: str) -> bool:
    t = (text or "").lower()
    return (
        ("root" in t and ("dir" in t or "directory" in t))
        or ("persist" in t and ("disk" in t or "saved" in t or "write" in t))
        or ("where" in t and ("memory" in t or "database" in t) and ("stored" in t or "saved" in t))
    )


def handle_root_query(mem: MemoryBridge) -> None:
    import os
    try:
        root = mem.root()
    except Exception as e:
        print(f"❌ Could not resolve root: {e}")
        return
    print(f"\n📂 Root: {root}")
    try:
        # Basic persistence checks
        cache_db = os.path.join(root, "cache", "memory.db")
        dag_nodes = os.path.join(root, "dag", "nodes")
        archive_dir = os.path.join(root, "archive")
        logbook = os.path.join(root, "logbook")
        ok_db = os.path.exists(cache_db)
        ok_dag = os.path.isdir(dag_nodes)
        ok_arch = os.path.isdir(archive_dir)
        ok_log = os.path.isdir(logbook)
        print("✅ Persistence checks:")
        print(f"   • DB exists: {ok_db} ({cache_db})")
        print(f"   • DAG nodes dir: {ok_dag} ({dag_nodes})")
        print(f"   • Archive dir: {ok_arch} ({archive_dir})")
        print(f"   • Logbook dir: {ok_log} ({logbook})")
    except Exception as e:
        print(f"❌ Error verifying persistence: {e}")
    try:
        st = mem.stats(None) or {}
        if int(st.get("archived", 0)) == 0:
            print("  ⓘ No archived items detected yet. Add a few more items or run :demo to populate archive/DAG.")
    except Exception:
        pass


def looks_like_recall_sources_query(text: str) -> bool:
    t = (text or "").lower()
    if "recall" in t and "source" in t:
        return True
    if ("what we" in t or "everything we" in t) and ("discussed" in t or "talked" in t) and "source" in t:
        return True
    return False


def handle_recall_sources_query(mem: MemoryBridge, user: str) -> None:
    import re
    t = (user or "").strip()
    m = re.search(r"about\s+(.+)$", t, flags=re.IGNORECASE)
    topic_raw = m.group(1).strip() if m else ""
    topic_raw = re.split(r"\band\b", topic_raw, flags=re.IGNORECASE)[0].strip()

    stop = {
        "the","a","an","and","or","but","of","to","for","with","about","my","our","your","me","we",
        "tell","which","source","each","memory","came","from","problem","issue","discussed","talked"
    }
    keywords = [w for w in re.findall(r"[a-zA-Z0-9]+", topic_raw.lower()) if w and w not in stop]

    lobes = ["preferences", "solutions", "insights", "chat", "notes"]
    ids: list[str] = []
    for l in lobes:
        try:
            ids.extend(mem.recent(l, 30) or [])
        except Exception:
            pass
    seen: set[str] = set()
    ids = [i for i in ids if not (i in seen or seen.add(i))]
    results = mem.recall_many(ids, "auto")

    def score(text: str) -> int:
        if not keywords:
            return 1
        t = (text or "").lower()
        return sum(1 for k in keywords if k in t)

    scored: list[tuple[str, dict, int]] = []
    for rid, r in zip(ids, results):
        if isinstance(r, dict) and r.get("content"):
            s = score(r["content"])
            if s > 0 or not keywords:
                scored.append((rid, r, s))

    scored.sort(key=lambda x: x[2], reverse=True)

    print("\n🧾 Recall summary")
    if not scored:
        print("  (no matching memories; showing recent)")
        for rid, r in zip(ids[:5], results[:5]):
            if isinstance(r, dict) and r.get("content"):
                txt = (r["content"] or "").strip().replace("\n", " ")
                if len(txt) > 140:
                    txt = txt[:140] + "..."
                print(f"  • {rid[:18]}… [src={r.get('source','auto')}] {txt}")
        return

    for rid, r, _s in scored[:10]:
        txt = (r["content"] or "").strip().replace("\n", " ")
        if len(txt) > 140:
            txt = txt[:140] + "..."
        print(f"  • {rid[:18]}… [src={r.get('source','auto')}] {txt}")

def looks_like_action_plan(text: str) -> bool:
    t = (text or "").lower()
    if "action plan" in t:
        return True
    if ("based on everything" in t or "based on what" in t or "based on all" in t) and (
        "next steps" in t or "plan" in t or "roadmap" in t or "action" in t
    ):
        return True
    if "personalized" in t and ("plan" in t or "next steps" in t):
        return True
    return False


def handle_action_plan(mem: MemoryBridge, user: str, convo: List[Dict[str, str]]) -> None:
    """Prepare context for an action plan and append it to the conversation.

    - Collect recent memories across key lobes
    - Print a short recall summary for transparency
    - Add a compact context block to the convo with memory IDs and previews
    - Add a system nudge instructing the LLM to produce a concrete plan that references IDs
    """
    lobes = ["preferences", "solutions", "insights", "chat"]
    ids: list[str] = []
    for l in lobes:
        try:
            ids.extend(mem.recent(l, 10) or [])
        except Exception:
            pass
    # Deduplicate, keep order
    seen: set[str] = set()
    ids = [i for i in ids if not (i in seen or seen.add(i))]
    if not ids:
        return
    results = mem.recall_many(ids, "auto")

    print("\n🧾 Recall summary")
    details: list[str] = []
    for rid, r in zip(ids[:12], results[:12]):
        if isinstance(r, dict) and r.get("content"):
            txt = (r["content"] or "").strip().replace("\n", " ")
            if len(txt) > 120:
                txt = txt[:120] + "..."
            src = r.get("source", "auto")
            print(f"  • {rid[:18]}… [src={src}] {txt}")
            details.append(f"{rid}:{src}:{txt}")

    if details:
        # Compact context to guide the LLM without overwhelming it
        context = "[Planning context: using memories =>\n" + "\n".join(details[:10]) + "]"
        convo.append({"role": "user", "content": context})
        convo.append({
            "role": "system",
            "content": (
                "Create a concise, personalized action plan with specific next steps. "
                "Reference the exact memory IDs inline where relevant (e.g., [preferences_ab12…]). "
                "Structure with short headings and numbered steps. Include owners, timing, and checks for safety and resource constraints."
            ),
        })
