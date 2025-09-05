import os
from typing import List, Dict

from memory_bridge import MemoryBridge


def tail_file(path: str, n: int = 3) -> list[str]:
    try:
        with open(path, 'r') as f:
            lines = f.readlines()
        return [ln.rstrip('\n') for ln in lines[-n:]]
    except Exception:
        return []


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
    print("✅ Demo complete. Continue chatting!")

