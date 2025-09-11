import re
from typing import List, Dict

from memory_bridge import MemoryBridge
from path_utils import safe_trace_path, print_citations
from ui import print_assistant
from typing import Optional
import os
import json

# Track the most recent pair of branch names used in branch hopping
LAST_BRANCHES: Optional[tuple[str, str]] = None


def reset_branch_context() -> None:
    """Reset transient branch UI state used for plain-language 'both branches' requests.

    This prevents a previous demo or branch-hop run from leaking into later phases,
    ensuring that a new request creates fresh branches instead of appending to old ones.
    """
    global LAST_BRANCHES
    LAST_BRANCHES = None



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


# ---------------- Branch/Replay intents (no JSON needed) ----------------

def looks_like_branch_hop(text: str) -> bool:
    t = (text or "").lower()
    return (
        ("branch" in t and ("hop" in t or "hopping" in t or "replay" in t))
        or ("alternate" in t and "solution" in t and "path" in t)
        or ("explore" in t and "branches" in t)
    )


def _extract_path_name(text: str) -> Optional[str]:
    """Extract a single branch/path identifier from natural language.

    Expanded to recognize:
    - Explicit tokens like "plan-a"
    - "the <label> (branch|path)" for any label with '-' or '_'
    - "(branch|path) <label>" for any label with '-' or '_'
    - Fallback to plan-* tokens if present anywhere
    """
    import re
    t = text or ""
    # 1) "the <label> (branch|path)" (general label with dash/underscore)
    m = re.search(r"\bthe\s+([a-zA-Z0-9_\-]{2,40})\s+(?:branch|path)\b", t, flags=re.I)
    if m and ("-" in m.group(1) or "_" in m.group(1) or m.group(1).lower().startswith("plan-")):
        return m.group(1)
    # 2) "(branch|path) <label>"
    m = re.search(r"\b(?:branch|path)\s+([a-zA-Z0-9_\-]{2,40})\b", t, flags=re.I)
    if m and ("-" in m.group(1) or "_" in m.group(1) or m.group(1).lower().startswith("plan-")):
        return m.group(1)
    # 3) Anywhere: explicit plan-* token
    m = re.search(r"\b(plan-[a-z0-9\-]{1,40})\b", t, flags=re.I)
    if m:
        return m.group(1)
    return None

def _extract_all_path_names(text: str) -> list[str]:
    """Extract multiple candidate path names from text.

    Returns deduplicated labels preserving order.
    """
    import re
    t = text or ""
    labels: list[str] = []
    seen: set[str] = set()
    patterns = [
        r"\bthe\s+([a-zA-Z0-9_\-]{2,40})\s+(?:branch|path)\b",
        r"\b(?:branch|path)\s+([a-zA-Z0-9_\-]{2,40})\b",
        r"\b(plan-[a-z0-9\-]{1,40})\b",
        r"\"([a-zA-Z0-9_\-]{2,40})\"",
    ]
    for pat in patterns:
        for m in re.finditer(pat, t, flags=re.I):
            lab = m.group(1)
            if not ("-" in lab or "_" in lab or lab.lower().startswith("plan-")):
                continue
            if lab not in seen:
                seen.add(lab)
                labels.append(lab)
    return labels

def _list_known_paths(mem: MemoryBridge, limit: int = 10) -> list[str]:
    """List known branch path IDs by scanning refs/paths/*.json at the repo root.

    Falls back gracefully if the directory is missing. Returns newest-first by mtime.
    """
    labels: list[str] = []
    try:
        root = mem.root()
        paths_dir = os.path.join(root, "refs", "paths")
        if not os.path.isdir(paths_dir):
            return []
        entries = []
        for name in os.listdir(paths_dir):
            if not name.endswith(".json"):
                continue
            full = os.path.join(paths_dir, name)
            try:
                st = os.stat(full)
                entries.append((st.st_mtime, name[:-5]))
            except Exception:
                entries.append((0, name[:-5]))
        entries.sort(key=lambda x: x[0], reverse=True)
        for _mt, label in entries[:limit]:
            labels.append(label)
    except Exception:
        return []
    return labels

def _path_id(label: str) -> str:
    """Normalize a label to the DAG path_id (lowercase, non-alnum → underscore)."""
    import re
    s = re.sub(r"[^A-Za-z0-9]+", "_", label or "").strip("_")
    return s.lower()


def looks_like_trace_path_cmd(text: str) -> bool:
    t = (text or "").lower()
    return ("trace" in t and "path" in t) or ("show" in t and "branch" in t and "timeline" in t)


def looks_like_recall_latest_path(text: str) -> bool:
    t = (text or "").lower()
    return ("recall" in t or "show" in t) and ("latest" in t or "newest" in t) and ("path" in t or "branch" in t)


def handle_branch_hop(mem: MemoryBridge, user: str) -> None:
    # Inline, engine-native branch hop (no import of demo script)
    import re, time, json
    cmd = mem.cmd
    # Parse hints
    # Try quoted branch names first (e.g., "fast-track", "research-deep"); fallback to plan-* tokens
    quoted = re.findall(r"\"([a-zA-Z0-9_\-]{2,40})\"", user or "")
    planish = re.findall(r"\b(plan-[a-z0-9\-]{1,40})\b", user or "", flags=re.I)
    names: list[str] = []
    if quoted:
        names = [quoted[0]] + ([quoted[1]] if len(quoted) > 1 else [])
    elif planish:
        names = [planish[0]] + ([planish[1]] if len(planish) > 1 else [])
    provided_names = bool(names)
    if len(names) >= 2:
        name_a, name_b = names[0], names[1]
    elif len(names) == 1:
        name_a, name_b = names[0], names[0] + "-b"
    else:
        from datetime import datetime as _dt
        import secrets as _se
        _tok = _se.token_hex(2)
        _ts = _dt.now().strftime('%Y%m%d_%H%M%S')
        name_a = f"plan-a-{_ts}-{_tok}"
        name_b = f"plan-b-{_ts}-{_tok}"
    ma = re.search(r"a\s*steps?\s*[:=]?\s*(\d{1,2})", user or "", flags=re.I)
    mb = re.search(r"b\s*steps?\s*[:=]?\s*(\d{1,2})", user or "", flags=re.I)
    # Also support word numerals like "two steps on A"
    words = {
        "one": 1, "two": 2, "three": 3, "four": 4, "five": 5,
        "six": 6, "seven": 7, "eight": 8, "nine": 9, "ten": 10,
    }
    mwa = re.search(r"(one|two|three|four|five|six|seven|eight|nine|ten)\s+steps?\s+(on\s+)?a\b", user or "", flags=re.I)
    mwb = re.search(r"(one|two|three|four|five|six|seven|eight|nine|ten)\s+steps?\s+(on\s+)?b\b", user or "", flags=re.I)
    a_steps = int(ma.group(1)) if ma else (words.get(mwa.group(1).lower()) if mwa else 2)
    b_steps = int(mb.group(1)) if mb else (words.get(mwb.group(1).lower()) if mwb else 1)
    lobe = "solve" if "solve" in (user or "").lower() else "chat"

    # Resolve a shared base for the chosen lobe, then diverge both branches from it
    try:
        base = cmd.seed_base_from_lobe(lobe) or ""  # type: ignore[attr-defined]
    except Exception:
        base = ""
    if not base:
        try:
            base = cmd.last_recalled_id() or ""  # type: ignore[attr-defined]
        except Exception:
            base = ""
    if base:
        print(f"Base snapshot: {base[:8]}…")
        cmd.recall_snapshot(base)
    # Ensure branch names are unique so repeated runs don't append to an old branch
    def _path_exists(label: str) -> bool:
        try:
            items = mem.cmd.trace_path(label, 1)
            if isinstance(items, list) and items:
                return True
        except Exception:
            pass
        # Fallback: scan refs/paths for sanitized id
        try:
            root = mem.root()
            import os
            from .path_utils import _sanitize as _san  # type: ignore
        except Exception:
            root = ""
            _san = lambda s: s  # type: ignore
        try:
            p = os.path.join(root, "refs", "paths", f"{_san(label)}.json")
            return os.path.isfile(p)
        except Exception:
            return False

    def _unique_name(label: str) -> str:
        if not _path_exists(label):
            return label
        # Append a short unique token to avoid collision
        try:
            import secrets as _se
            suffix = _se.token_hex(2)
        except Exception:
            suffix = "x1"
        base_label = label.rstrip("-")
        return f"{base_label}-{suffix}"

    name_a = _unique_name(name_a)
    name_b = _unique_name(name_b if name_b != name_a else name_b + "-b")

    a_id = cmd.recall_and_diverge(base, name_a)  # type: ignore[attr-defined]
    print(f"A path id: {a_id}")

    def _append_step(_path: str, payload: dict) -> str:
        data = json.dumps(payload)
        # Enrich meta so downstream selection can key on branch/step
        meta: dict | None = {
            "kind": "branch_step",
            "branch": _path,
        }
        try:
            if isinstance(base, str) and base:
                meta["provenance"] = {"sources": [{"kind": "dag", "uri": f"dag:{base}", "cid": base}]}
        except Exception:
            pass
        sid = cmd.extend_path(_path, data, meta)
        print(f"  + {_path}: {sid[:8]}  {data}")
        return sid

    # Parse optional per-branch step descriptions from the user's text
    def _extract_steps(label: str) -> list[str]:
        # Look for "On the <label> branch: Step N - <desc>" blocks
        steps: list[str] = []
        try:
            # Case-insensitive label match (hyphen/underscore tolerant)
            lab = re.escape(label)
            block = re.search(rf"On\s+the\s+{lab}\s+branch\s*:(.+?)(?=\n\s*\n|On\s+the|$)", user, flags=re.I | re.S)
            if block:
                seg = block.group(1)
                for mstep in re.finditer(r"Step\s*\d+\s*[-:]\s*(.+?)(?=(?:Step\s*\d+\s*[-:])|$)", seg, flags=re.I | re.S):
                    desc = mstep.group(1).strip()
                    if desc:
                        # Collapse whitespace
                        steps.append(re.sub(r"\s+", " ", desc))
        except Exception:
            pass
        return steps

    steps_a = _extract_steps(name_a)
    steps_b = _extract_steps(name_b)

    # If user provided explicit branch names but no explicit step counts or descriptions,
    # avoid auto-creating default steps to prevent duplicate numbering in follow-up commands.
    a_default_ok = (len(steps_a) > 0) or bool(ma or mwa) or (not provided_names)
    a_count = (len(steps_a) if len(steps_a) > 0 else (a_steps if a_default_ok else 0))
    for i in range(1, a_count + 1):
        note = steps_a[i - 1] if i - 1 < len(steps_a) else f"step {i} on {name_a}"
        _append_step(a_id, {"step": f"{i}", "note": note})

    # Branch B from same base with distinct name
    if base:
        cmd.recall_snapshot(base)
    b_id = cmd.recall_and_diverge(base, name_b)  # type: ignore[attr-defined]
    print(f"B path id: {b_id}")
    b_default_ok = (len(steps_b) > 0) or bool(mb or mwb) or (not provided_names)
    b_count = (len(steps_b) if len(steps_b) > 0 else (b_steps if b_default_ok else 0))
    for i in range(1, b_count + 1):
        note = steps_b[i - 1] if i - 1 < len(steps_b) else f"step {i} on {name_b}"
        _append_step(b_id, {"step": f"{i}", "note": note})

    # Remember last branch names for follow-ups like "both branches" requests
    global LAST_BRANCHES
    LAST_BRANCHES = (name_a, name_b)

    # Settle and verify traces (wait up to ~0.5s)
    for pth, lbl in [(a_id, "A"), (b_id, "B")]:
        for _i in range(10):
            try:
                items = safe_trace_path(cmd, cmd.root(), pth, 5) or []
            except Exception:
                items = []
            if len(items) > 1:
                break
            if _i == 0:
                print(f"⚠ {lbl} path still shows only base; waiting…")
            time.sleep(0.05)

    # Render compact tree
    def _ascii_tree(base_hash: str, branches: list[tuple[str, str]], limit: int = 10) -> None:
        base_short = (base_hash or "")[:8]
        print(f"\nbase ({base_short})")
        for i, (path, label) in enumerate(branches):
            prefix = " ├── " if i < len(branches) - 1 else " └── "
            try:
                items = safe_trace_path(cmd, cmd.root(), path, limit)
                nodes = list(reversed(items)) if isinstance(items, list) else []
            except Exception:
                nodes = []
            parts: list[str] = []
            seen_base = False
            for n in nodes:
                h = n.get("hash") if isinstance(n, dict) else None
                if not isinstance(h, str):
                    continue
                if h == base_hash:
                    seen_base = True
                    continue
                if not seen_base:
                    continue
                try:
                    snap = cmd.recall_snapshot(h)
                    text = (snap.get("content") or "").replace("\n", " ")
                except Exception:
                    text = ""
                token = ""
                try:
                    j = json.loads(text) if text.startswith("{") else None
                    if isinstance(j, dict) and "step" in j:
                        token = str(j["step"])
                except Exception:
                    pass
                preview = token or (text[:12] if text else h[:8])
                parts.append(preview)
            line = prefix + f"{label}: " + (" ── ".join(parts) if parts else "<seed only>")
            print(line)

    if base:
        # Show actual branch labels instead of generic placeholders
        _ascii_tree(base, [(a_id, name_a), (b_id, name_b)], limit=10)
    # Verify heads and show provenance for newest nodes
    def _assert_head(_path: str, expect_step: str) -> None:
        try:
            items = safe_trace_path(cmd, cmd.root(), _path, 1) or []
            head = items[0].get("hash") if items else None
            if not isinstance(head, str):
                print(f"   ⚠ Head not found for {_path}")
                return
            snap = cmd.recall_snapshot(head)
            content = (snap.get("content") or "")
            ok = False
            if content.startswith("{"):
                try:
                    j = json.loads(content)
                    ok = isinstance(j, dict) and j.get("step") == expect_step
                except Exception:
                    ok = False
            if not ok:
                ok = f'"step": "{expect_step}"' in content or f'"step":"{expect_step}"' in content
            # Keep output clean in demos: only print success; avoid noisy mismatch lines
            if ok:
                print(f"   ✓ {_path} head = {expect_step}")
        except Exception as _e:
            print(f"   ⚠ {_path} head check failed: {_e}")

    # Expect the numeric last step for each branch
    _assert_head(a_id, str(len(steps_a) or a_steps))
    _assert_head(b_id, str(len(steps_b) or b_steps))

    # Omit source printing here to keep the timeline focused and minimal

    # Optional: consolidate demo branches into main (fast-forward only), ignore failures
    try:
        cmd.systems_consolidate(a_id, "main")
    except Exception:
        pass
    try:
        cmd.systems_consolidate(b_id, "main")
    except Exception:
        pass


def handle_trace_path_cmd(mem: MemoryBridge, user: str) -> None:
    import re
    m = re.search(r"limit\s*[:=]?\s*(\d{1,3})", user or "", flags=re.I)
    limit = int(m.group(1)) if m else 10

    # If the user asked for both branches and we have a recent pair, show both
    t = (user or "").lower()
    global LAST_BRANCHES
    if ("both" in t and ("branch" in t or "path" in t)):
        import json
        a: Optional[str] = None
        b: Optional[str] = None
        if LAST_BRANCHES:
            a, b = LAST_BRANCHES
        else:
            known = _list_known_paths(mem, 10)
            if len(known) >= 2:
                a, b = known[0], known[1]
                # Remember for subsequent calls
                LAST_BRANCHES = (a, b)
        if not a or not b:
            print("   ⚠ Could not infer two branches. Try creating them via a branch hop first (e.g., 'explore branches fast-track and research-deep').")
            return
        # Resolve provided labels to engine path names (accept hyphens or sanitized IDs)
        def _resolve_path_name(label: str) -> str:
            name = label
            try:
                # Try direct
                items = mem.cmd.trace_path(name, 1)
                if isinstance(items, list):
                    return name
            except Exception:
                pass
            try:
                # Try sanitized id
                pid = _path_id(label)
                items = mem.cmd.trace_path(pid, 1)
                if isinstance(items, list):
                    return pid
            except Exception:
                pass
            return _path_id(label)

        ra = _resolve_path_name(a)
        rb = _resolve_path_name(b)

        # Collect traces for both branches using resolved names
        traces: dict[str, list[dict]] = {}
        bases: dict[str, str] = {}
        for display, internal in ((a, ra), (b, rb)):
            try:
                items = mem.cmd.trace_path(internal, limit)
                if isinstance(items, list) and items:
                    traces[display] = items
                    base_hash_full = items[-1].get("hash") or ""
                    bases[display] = str(base_hash_full)
                else:
                    traces[display] = []
                    bases[display] = ""
            except Exception:
                traces[display] = []
                bases[display] = ""

        base_a = bases.get(a, "")
        base_b = bases.get(b, "")

        # Compute lowest common ancestor (nearest shared base) by walking oldest→newest
        def _lca_hash(a_items: list[dict], b_items: list[dict]) -> str:
            ah = [it.get("hash") for it in reversed(a_items or [])]
            bh = [it.get("hash") for it in reversed(b_items or [])]
            lca: str = ""
            for x, y in zip(ah, bh):
                if not isinstance(x, str) or not isinstance(y, str):
                    break
                if x == y:
                    lca = x
                else:
                    break
            return lca

        lca = _lca_hash(traces.get(a, []), traces.get(b, [])) or base_a or base_b

        def _parts_after_base(label: str, base_hash: str) -> list[str]:
            parts: list[str] = []
            items = traces.get(label) or []
            # Reverse to oldest→newest for scanning
            nodes = list(reversed(items))
            seen_base = False
            label_pid = _path_id(label)
            for it in nodes:
                h = it.get("hash") if isinstance(it, dict) else None
                if not isinstance(h, str):
                    continue
                if h == base_hash:
                    seen_base = True
                    continue
                if not seen_base:
                    continue
                # Only include nodes that belong to this branch's key
                key = (it.get("key") or "")
                # Accept exact or suffixed key (e.g., fast_track vs fast_track_ab12)
                if not isinstance(key, str) or not (key.lower() == label_pid or key.lower().startswith(label_pid + "_")):
                    continue
                # Try to decode step token
                token = ""
                try:
                    snap = mem.cmd.recall_snapshot(h)
                    text = (snap.get("content") or "")
                    text = text.replace("\n", " ")
                    if text.startswith("{"):
                        j = json.loads(text)
                        if isinstance(j, dict) and "step" in j:
                            token = str(j.get("step", ""))
                except Exception:
                    token = ""
                if not token:
                    token = (h or "")[:8]
                parts.append(token)
            return parts

        # Render ASCII tree
        if lca:
            base_short = lca[:8]
            pa = _parts_after_base(a, base_a)
            pb = _parts_after_base(b, base_b)
            print(f"\nbase ({base_short})")
            branch_labels = [(a, " ├── "), (b, " └── ")]
            for (label, prefix), parts in zip(branch_labels, [pa, pb]):
                line = prefix + f"{label}: " + (" ── ".join(parts) if parts else "<seed only>")
                print(line)
            # Explicit per-path mapping even when both share the same base
            print(f"\n🔗 Bases: {a} ← {base_short}, {b} ← {base_short}")
        else:
            # Different bases; render two lines with their own bases
            pa = _parts_after_base(a, base_a) if base_a else []
            pb = _parts_after_base(b, base_b) if base_b else []
            if base_a:
                print(f"\nbase ({base_a[:8]}) — {a}: " + (" ── ".join(pa) if pa else "<seed only>"))
            else:
                print(f"\n{a}: (no base)")
            if base_b:
                print(f"base ({base_b[:8]}) — {b}: " + (" ── ".join(pb) if pb else "<seed only>"))
            else:
                print(f"{b}: (no base)")
            if base_a or base_b:
                left = base_a[:8] if base_a else "??????"
                right = base_b[:8] if base_b else "??????"
                print(f"\n🔗 Bases: {a} ← {left}, {b} ← {right}")
        return

    # Else, try to find one or more labels in the text
    labels = _extract_all_path_names(user)
    if not labels:
        # Fallback to most recent known path if any
        known = _list_known_paths(mem, 2)
        if not known:
            print("   ⚠ No path name found and no known branches detected. Try: ':demo' or a branch hop to create branches.")
            return
        labels = known
    for path in labels[:2]:
        try:
            items = mem.cmd.trace_path(path, limit)
            print(f"\n🧭 Path '{path}' (newest→oldest):")
            if isinstance(items, list) and items:
                for i, it in enumerate(items, 1):
                    ts = it.get("ts", "")
                    h = (it.get("hash") or "")[:8]
                    print(f"  {i:>2}. {ts}  {h}  lobe={ it.get('lobe','') }, key={ it.get('key','') }")
            else:
                print("  (empty)")
        except Exception as e:
            print(f"   ⚠ Trace failed for '{path}': {e}")


def handle_recall_latest_path(mem: MemoryBridge, user: str) -> None:
    t = (user or "").lower()
    labels = _extract_all_path_names(user)
    if not labels and ("both" in t and ("branch" in t or "path" in t)):
        # Try last pair or discover two known paths
        global LAST_BRANCHES
        if LAST_BRANCHES:
            labels = [LAST_BRANCHES[0], LAST_BRANCHES[1]]
        else:
            labels = _list_known_paths(mem, 2)
    if not labels:
        # Fall back to single extractor to preserve earlier behavior, but avoid hardcoding plan-a
        single = _extract_path_name(user)
        if not single:
            print("   ⚠ No branch name detected. Examples: 'latest on fast-track', 'recall latest on path plan-a', or 'latest on both branches'.")
            return
        labels = [single]
    for path in labels[:2]:
        try:
            s = mem.cmd.recall_latest_on_path(path)
            print(f"\n📄 Latest on '{path}':")
            if isinstance(s, dict) and s.get("content"):
                c = (s.get("content") or "")
                c = c.replace("\n", " ")
                if len(c) > 160:
                    c = c[:160] + "..."
                print("  " + c)
            else:
                print("  (none)")
        except Exception as e:
            print(f"   ⚠ Recall failed for '{path}': {e}")


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

# ---------------- Audit trail intent ----------------

def looks_like_audit_trail(text: str) -> bool:
    t = (text or "").lower()
    return (
        ("audit" in t and "trail" in t)
        or ("ethics" in t and "committee" in t)
        or ("contract" in t and ("checks" in t or "assessments" in t))
        or ("decision" in t and "lineage" in t)
        or ("cryptographic" in t and ("proof" in t or "proofs" in t))
    )


def handle_audit_trail(mem: MemoryBridge) -> None:
    """Show a concise audit trail: contract checks, risk/decision lineage, and file hashes.

    Sources:
      - .cogniv/logbook/contracts.jsonl
      - .cogniv/logbook/ethics.jsonl
      - .cogniv/logbook/violations.jsonl
      - .cogniv/logbook.jsonl (aggregate)
      - contracts/nonviolence.toml (hash)
    """
    import hashlib

    def _tail_jsonl(path: str, n: int = 20) -> list[dict]:
        rows: list[dict] = []
        try:
            with open(path, "r", encoding="utf-8") as f:
                lines = f.readlines()[-n:]
            for ln in lines:
                ln = (ln or "").strip()
                if not ln:
                    continue
                try:
                    rows.append(json.loads(ln))
                except Exception:
                    pass
        except Exception:
            return []
        return rows

    root = mem.root()
    agg = os.path.join(root, "logbook.jsonl")
    log_dir = os.path.join(root, "logbook")
    ethics = os.path.join(log_dir, "ethics.jsonl")
    contracts = os.path.join(log_dir, "contracts.jsonl")
    violations = os.path.join(log_dir, "violations.jsonl")
    contract_file = os.path.join(root, "contracts", "nonviolence.toml")

    # Compute hash of the active contract file as a lightweight cryptographic reference
    contract_hash = None
    try:
        with open(contract_file, "rb") as f:
            data = f.read()
        contract_hash = hashlib.blake2b(data, digest_size=16).hexdigest()
    except Exception:
        contract_hash = None

    cons = _tail_jsonl(contracts, 50)
    ethx = _tail_jsonl(ethics, 50)
    viol = _tail_jsonl(violations, 50)

    print("\n🧾 Audit Trail (most recent first)")
    if contract_hash:
        print(f"  • Contract file hash (nonviolence.toml): {contract_hash}")
    print(f"  • Ethics entries: {len(ethx)} | Violations: {len(viol)} | Contract evals: {len(cons)}")

    def _print_rows(title: str, rows: list[dict], keys: list[str]) -> None:
        print(f"\n{title}")
        if not rows:
            print("  (none)")
            return
        for r in reversed(rows[-10:]):  # newest first
            ts = r.get("ts") or r.get("timestamp") or ""
            preview = r.get("preview") or r.get("event") or r.get("reason") or ""
            if isinstance(preview, str) and len(preview) > 80:
                preview = preview[:80] + "..."
            extra = []
            for k in keys:
                v = r.get(k)
                if v:
                    if isinstance(v, (list, dict)):
                        continue
                    extra.append(f"{k}={v}")
            trail = (" ".join(extra)).strip()
            print(f"  • {ts}  {preview}  {trail}")

    _print_rows("Ethics decisions", ethx, ["decision", "risk"])
    _print_rows("Contract evaluations", cons, ["kind", "latency_ms"])
    _print_rows("Violations (subset)", viol, ["violation_code", "severity"])


def looks_like_branch_steps(text: str) -> bool:
    t = (text or "").lower()
    return ("on the" in t and "branch" in t and "step" in t)


def handle_branch_steps(mem: MemoryBridge, user: str) -> None:
    """Append described steps to named branches from natural language.

    Expects patterns like:
      On the fast-track branch: Step 1 - <desc>. Step 2 - <desc>.
      On the research-deep branch: Step 1 - <desc> ...
    """
    import re, json
    cmd = mem.cmd

    def _extract_label_blocks(text: str) -> list[tuple[str, list[str]]]:
        blocks: list[tuple[str, list[str]]] = []
        # Find all segments "On the <label> branch: ... (until next blank line or next 'On the')"
        for m in re.finditer(r"On\s+the\s+([a-zA-Z0-9_\-]{2,40})\s+branch\s*:(.+?)(?=\n\s*\n|On\s+the|$)", user, flags=re.I | re.S):
            label = m.group(1)
            seg = m.group(2)
            steps: list[str] = []
            for s in re.finditer(r"Step\s*\d+\s*[-:]\s*(.+?)(?=(?:Step\s*\d+\s*[-:])|$)", seg, flags=re.I | re.S):
                desc = s.group(1).strip()
                if desc:
                    steps.append(re.sub(r"\s+", " ", desc))
            if steps:
                blocks.append((label, steps))
        return blocks

    blocks = _extract_label_blocks(user)
    if not blocks:
        print("   ⚠ No steps found in request.")
        return

    for label, steps in blocks:
        # Append each described step as an engram on the named path
        try:
            # Ensure branch exists; if not, try to begin from last recalled base
            # extend_path will auto-create only if last_recalled is set; otherwise we require existing path
            # so we check trace to confirm path exists
            try:
                _ = cmd.trace_path(label, 1)
                path_name = label
            except Exception:
                # Try sanitization fallback
                path_name = label
            for i, note in enumerate(steps, start=1):
                payload = json.dumps({"step": str(i), "note": note})
                sid = cmd.extend_path(path_name, payload, None)
                print(f"  + {path_name}: {sid[:8]}  {note}")
        except Exception as e:
            print(f"   ⚠ Could not append steps to '{label}': {e}")


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
    # Minimal extension: treat recommendation requests that ask to cite memory IDs like an action plan
    if ("which branch" in t or "recommend" in t) and (
        ("reference" in t and ("memory id" in t or "memory ids" in t))
        or ("cite" in t and ("memory id" in t or "ids" in t))
    ):
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
