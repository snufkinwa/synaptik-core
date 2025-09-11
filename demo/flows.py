import os
from typing import List, Dict

from memory_bridge import MemoryBridge
from path_utils import safe_trace_path, print_citations
import json


def tail_file(path: str, n: int = 3) -> list[str]:
    try:
        with open(path, 'r') as f:
            lines = f.readlines()
        return [ln.rstrip('\n') for ln in lines[-n:]]
    except Exception:
        return []


def ascii_tree(cmd, base_hash: str, branches: list[tuple[str, str]], limit: int = 10) -> None:
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


def run_demo_flow(mem: MemoryBridge) -> None:
    print("\n🚀 Running scripted demo...")
    root = mem.root()
    print(f"   Root: {root}")

    # 1) Persist a preference
    pref_text = "User prefers short, friendly greetings"
    pref_id = mem.remember("preferences", pref_text, "user_pref")
    print(f"   💾 Saved preference id: {pref_id[:24]}...")

    # 2) Ensure chat lobe reaches 5 to trigger auto-promotion
    chat_stats_before = mem.stats("chat")
    before_total = chat_stats_before.get('total', 0)
    before_arch = chat_stats_before.get('archived', 0)
    before_hot = max(0, before_total - before_arch)
    need = max(0, 5 - before_hot)
    for i in range(need):
        mem.remember("chat", f"demo chat note {i+1}", None)
    chat_stats_after = mem.stats("chat")
    print(f"   📊 Chat before: total={before_total}, archived={before_arch}")
    print(f"   📊 Chat after:  total={chat_stats_after.get('total',0)}, archived={chat_stats_after.get('archived',0)}")

    # Show filesystem archive objects written under .cogniv/archive
    try:
        arch_dir = os.path.join(root, 'archive')
        if os.path.isdir(arch_dir):
            objs = []
            for name in os.listdir(arch_dir):
                p = os.path.join(arch_dir, name)
                if os.path.isfile(p) and len(name) >= 16:
                    objs.append(name)
            print(f"   📦 Archive objects: {len(objs)} in .cogniv/archive/")
            if objs:
                sample = ", ".join(objs[:2])
                print(f"      e.g.: {sample}")
    except Exception:
        pass

    # 3) Pick a recent chat memory and show its recall source (auto), then force DAG recall
    chat_ids = mem.recent("chat", 1)
    if chat_ids:
        rid = chat_ids[0]
        r_auto = mem.recall(rid, "auto")
        if isinstance(r_auto, dict):
            src = r_auto.get('source', 'auto')
            prev = (r_auto.get('content') or '')[:80]
            print(f"   🔎 Recall(auto) {rid[:18]}... source={src}, content='{prev}'")
        r_dag = mem.recall(rid, "dag")
        if isinstance(r_dag, dict) and r_dag.get('content'):
            prev = (r_dag.get('content') or '')[:80]
            print(f"   🧩 Recall(dag)  {rid[:18]}... source=dag, content='{prev}'")

    # 4) Lobe separation: preference vs solution
    mem.remember("solutions", "Final answer: 42 because constraints...", "solution_1")
    pref_recent = (mem.recent("preferences", 1) or [None])[0]
    sol_recent = (mem.recent("solutions", 1) or [None])[0]
    if pref_recent:
        rp = mem.recall(pref_recent, "auto")
        if isinstance(rp, dict):
            print(f"   📁 preferences → {rp.get('content','')[:60]}")
    if sol_recent:
        rs = mem.recall(sol_recent, "auto")
        if isinstance(rs, dict):
            print(f"   📁 solutions   → {rs.get('content','')[:60]}")

    # 5) Ethics precheck and audit tail
    res = mem.cmd.precheck_text("I want to kill her", "chat_message")
    decision = res.get('decision','?')
    risk = res.get('risk','?')
    print(f"   🛡️ Precheck: {decision.upper()} (risk={risk})")
    ethics_log = os.path.join(root, 'logbook', 'ethics.jsonl')
    tail = tail_file(ethics_log, 3)
    if tail:
        print("   📜 Ethics log tail:")
        for ln in tail:
            print("      " + ln)
    # 6) Branch hopping demo (replay): explore two paths and render ASCII timelines
    try:
        print("\n🌿 Branch hopping (replay mode)...")
        cmd = mem.cmd
        # Branch names: generate unique ids to avoid collisions and stale refs
        from datetime import datetime as _dt
        import secrets as _se
        # Inline secrets token generation (8 hex chars)
        _tok = _se.token_hex(2)
        _ts = _dt.now().strftime('%Y%m%d_%H%M%S')
        name_a = f"plan-a-{_ts}-{_tok}"
        name_b = f"plan-b-{_ts}-{_tok}"

        # Resolve a shared base snapshot for the 'chat' lobe
        try:
            base = cmd.seed_base_from_lobe("chat") or ""  # type: ignore[attr-defined]
        except Exception:
            base = ""
        if not base:
            # Fallback: try last recalled
            try:
                base = cmd.last_recalled_id() or ""  # type: ignore[attr-defined]
            except Exception:
                base = ""
        if base:
            print(f"Base snapshot: {base[:8]}…")
            cmd.recall_snapshot(base)
        # Diverge A from the known base
        a_id = cmd.recall_and_diverge(base, name_a)  # type: ignore[attr-defined]
        print(f"A path id: {a_id}")

        # Append steps on A
        def _append_step(_cmd, _path, _payload):
            meta = None
            try:
                if isinstance(base, str) and base:
                    meta = {"provenance": {"sources": [{"kind": "dag", "uri": f"dag:{base}", "cid": base}]}}
            except Exception:
                meta = None
            sid = _cmd.extend_path(_path, _payload, meta)
            print(f"  + {_path}: {sid[:8]}  {_payload}")
            return sid

        last_a_step = None
        for i in (1, 2):
            last_a_step = str(i)
            _append_step(cmd, a_id, f'{{"step":"{last_a_step}","note":"step {last_a_step} on {name_a}"}}')

        # Branch B from the same base using distinct name
        if base:
            cmd.recall_snapshot(base)
        b_id = cmd.recall_and_diverge(base, name_b)  # type: ignore[attr-defined]
        print(f"B path id: {b_id}")

        # Append steps on B
        last_b_step = None
        for i in (1,):
            last_b_step = str(i)
            _append_step(cmd, b_id, f'{{"step":"{last_b_step}","note":"step {last_b_step} on {name_b}"}}')

        # Settle and verify
        import time as _t, json as _json
        # Wait up to ~0.5s for refs/index to settle
        for _pth, _lbl in [(a_id, "A"), (b_id, "B")]:
            for _i in range(10):
                try:
                    _items = safe_trace_path(cmd, cmd.root(), _pth, 5) or []
                except Exception:
                    _items = []
                if len(_items) > 1:
                    break
                if _i == 0:
                    print(f"⚠ {_lbl} path still shows only base; waiting…")
                _t.sleep(0.05)

        # Verify heads and render compact tree without importing demo module
        if base:
            ascii_tree(cmd, base, [(a_id, "plan_a"), (b_id, "plan_b")], limit=10)
        # Head verification helper
        def _assert_head(_path: str, expect_step: str) -> None:
            try:
                head = (cmd.trace_path(_path, 1) or [{}])[0].get("hash")
                if not isinstance(head, str):
                    print(f"   ⚠ Head not found for {_path}")
                    return
                snap = cmd.recall_snapshot(head)
                content = (snap.get("content") or "")
                ok = False
                if content.startswith("{"):
                    import json as _json
                    try:
                        j = _json.loads(content)
                        ok = isinstance(j, dict) and j.get("step") == expect_step
                    except Exception:
                        ok = False
                if not ok:
                    ok = f'"step": "{expect_step}"' in content or f'"step":"{expect_step}"' in content
                # Keep output clean: print success tick only
                if ok:
                    print(f"   ✓ {_path} head = {expect_step}")
            except Exception as _e:
                print(f"   ⚠ {_path} head check failed: {_e}")

        # Verify against the actual last steps we appended
        _assert_head(a_id, last_a_step or "1")
        _assert_head(b_id, last_b_step or "1")

        # Flash provenance for newest node on each branch
        for _p, _lbl in ((a_id, "plan_a"), (b_id, "plan_b")):
            try:
                head = (cmd.trace_path(_p, 1) or [{}])[0].get("hash")
                if isinstance(head, str):
                    print_citations(cmd, head, _lbl)
                else:
                    print(f"   🔗 Sources for {_lbl} @ ????????: []")
            except Exception as _e:
                print(f"   ⚠ Cite sources failed for {_lbl}: {_e}")
    except Exception as e:
        print(f"   ⚠ Branch demo skipped: {e}")


    ff_api(mem)

    print("✅ Demo complete. Continue chatting!")


def ff_api(mem: MemoryBridge) -> None:
    """Demonstrate PyCommands: sprout_dendrite, encode_engram, systems_consolidate,
    plus engram_head, set_engram_head, and reconsolidate_paths (FF placeholder).

    Creates a unique feature path, appends two steps, fast-forwards cortex, and shows head updates.
    """
    import secrets as _se
    from datetime import datetime as _dt

    cmd = mem.cmd
    print("\n🧠 Neuro-ops: sprout_dendrite → encode_engram → systems_consolidate (FF-only)")


    # Unique path names
    _tok = _se.token_hex(2)
    _ts = _dt.now().strftime('%Y%m%d_%H%M%S')
    feature = f"feature-x-{_ts}-{_tok}"
    scratch = f"scratch-{_ts}-{_tok}"
    feature2 = f"feature-y-{_ts}-{_tok}"

    try:
        # Sprout from head('cortex') if present; else seed from 'chat'
        base = cmd.sprout_dendrite(path=feature, base=None, lobe=None)
        print(f"   • sprout_dendrite(path={feature}) base={base[:8]}…")

        # Encode two engrams (Ethos-gated; meta auto-enriched in core)
        s1 = cmd.encode_engram(path=feature, content='{"step":"1","note":"explore X1"}', meta={"kind": "demo"})
        s2 = cmd.encode_engram(path=feature, content='{"step":"2","note":"explore X2"}', meta={"kind": "demo"})
        print(f"   • encode_engram → {s1[:8]}…, {s2[:8]}…")

        # Check head
        head = cmd.engram_head(feature) or ""
        print(f"   • engram_head({feature}) = {head[:8]}…")

        # Fast-forward systems consolidation to cortex
        new_cortex = cmd.systems_consolidate(src_path=feature, dst_path="cortex")
        print(f"   • systems_consolidate({feature} → cortex) = {new_cortex[:8]}…")

        # Create a new engram on the same feature after consolidation
        s3 = cmd.encode_engram(path=feature, content='{"step":"3","note":"extend X3"}', meta={"kind": "demo"})
        print(f"   • encode_engram → {s3[:8]}…")

        # Force-set a separate scratch path head to base (creates ref if missing)
        cmd.set_engram_head(scratch, base)
        scratch_head = cmd.engram_head(scratch) or ""
        print(f"   • set_engram_head({scratch} = base) → {scratch_head[:8]}…")

        # Reconsolidate cortex to the latest feature head (FF)
        merged = cmd.reconsolidate_paths(src_path=feature, dst_path="cortex", note="demo")
        print(f"   • reconsolidate_paths({feature} → cortex) = {merged[:8]}… (FF)")

    except Exception as e:
        print(f"   ⚠ New API demo skipped: {e}")
