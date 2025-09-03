import os
import re
from typing import List, Dict


from prompts import system_prompt
from llm_client import chat, MODEL
from actions import maybe_parse_action, route
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
                # CIDs are long hex strings; filter non-files or subdirs
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
        # Auto (may be hot/archive/dag depending on state)
        r_auto = mem.recall(rid, "auto")
        if isinstance(r_auto, dict):
            src = r_auto.get('source', 'auto')
            prev = (r_auto.get('content') or '')[:80]
            print(f"   🔎 Recall(auto) {rid[:18]}... source={src}, content='{prev}'")
        # Explicit DAG-only recall to demonstrate cold graph retrieval
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


def run_repl() -> None:
    mem = MemoryBridge()
    print("🧠 Synaptik Agent x Groq Responses API — Persistent Memory & Ethics")
    print(f"🤖 Model: {MODEL}")
    print(f"💾 Root: {mem.root()}")
    print()
    print("💡 This agent will:")
    print("   • Remember important information from our conversations")
    print("   • Build knowledge over time using Memory IDs")
    print("   • Check ethics before storing sensitive content")
    print("   • Reference previous conversations using Memory IDs")
    print("\nType ':demo' anytime to run a quick end-to-end demo.")
    print("=" * 60)
    # Track whether we've printed the chat area header yet
    posted_chat_header = False

    # Quick connectivity tests
    try:
        print("\n🧪 Testing APIs...")
        stats = mem.stats(None)
        print(f"✓ Synaptik Core: {stats['total']} memories")
    except Exception as e:
        print(f"⚠ Startup test failed: {e}")

    convo: List[Dict[str, str]] = [{"role": "system", "content": system_prompt()}]

    # Startup: load recent preferences context
    try:
        print("\n🧠 Loading recent memories...")
        recent_ids = mem.recent("preferences", 3)
        if recent_ids:
            startup_memories: list[str] = []
            name: str | None = None
            for mid in recent_ids:
                r = mem.recall(mid)
                if isinstance(r, dict) and r.get("content"):
                    text = r["content"][:200]
                    startup_memories.append(text)
                    if text.lower().startswith("user_name:"):
                        name = text.split(":", 1)[1].strip()
            if startup_memories:
                print("📚 Context from previous sessions:")
                for i, memory in enumerate(startup_memories):
                    preview = memory[:80] + "..." if len(memory) > 80 else memory
                    print(f"   {i+1}. {preview}")
                context_summary = "Previous context from stored memories: " + "; ".join(startup_memories)
                convo.append({"role": "user", "content": f"[{context_summary}]"})
                try:
                    # Ensure chat header appears before the first assistant message
                    if not posted_chat_header:
                        print("\n" + "-" * 60)
                        print("💬 Chat")
                        print("-" * 60)
                        posted_chat_header = True
                    assistant = chat(convo)
                    act = maybe_parse_action(assistant)
                    reasoning_text = assistant
                    if act:
                        m = re.search(r"\{[^{}]*\"action\"[^{}]*\}", assistant, flags=re.DOTALL)
                        if m:
                            reasoning_text = assistant[:m.start()].strip()
                    if reasoning_text:
                        print(f"🤖 {reasoning_text}")
                    if act:
                        result = route(act, mem)
                        action_name = act.get("action", "unknown")
                        print(f"\n🔧 Action: {action_name}")
                        if result.get("ok"):
                            print("✅ Success")
                        else:
                            print(f"❌ Failed: {result.get('error', 'Unknown error')}")
                    convo.append({"role": "assistant", "content": assistant})
                except Exception as e:
                    print(f"⚠ Startup greet failed: {e}")
            else:
                print("📝 No previous context found - starting fresh!")
    except Exception as e:
        print(f"⚠ Memory loading failed: {e}")

    # Print the chat header once before entering the REPL loop
    if not posted_chat_header:
        print("\n" + "-" * 60)
        print("💬 Chat")
        print("-" * 60)
        posted_chat_header = True

    while True:
        try:
            user = input("\nYou> ").strip()
        except (EOFError, KeyboardInterrupt):
            print("\nGoodbye!")
            break

        if user.lower() in {"exit", "quit", "q"}:
            break
        if not user:
            continue
        # Demo command triggers (accept several variants)
        if user.strip().lower() in {":demo", ":d", "demo", "/demo", "run demo", "start demo"}:
            try:
                run_demo_flow(mem)
            except Exception as e:
                print(f"❌ Demo error: {e}")
            continue

        # Run local precheck BEFORE hitting the LLM, so audit/ethics logs record
        try:
            pre = mem.cmd.precheck_text(user, "chat_message")
            decision = pre.get("decision", "allow")
            risk = pre.get("risk", "?")
            icon = {"allow":"✅","allow_with_constraints":"⚠️","block":"🚫"}.get(decision,"❓")
            print(f"🛡️ Precheck: {icon} {decision.upper()} | Risk: {risk}")
            # Show constraints (these guide the LLM on unsafe or constrained inputs)
            constraints = pre.get("constraints") or []
            if constraints:
                print(f"   constraints: {constraints}\n")
            # If soft-allowed with constraints, nudge the model by adding them as context
            if decision == "allow_with_constraints" and constraints:
                convo.append({"role": "user", "content": f"[Apply constraints: {', '.join(constraints)}]"})
            # If blocked, do not forward the raw input — call the LLM with a safety-specific prompt
            if decision == "block":
                guidance = (
                    "If user content is unsafe, do not repeat it. Instead: "
                    "reframe nonviolent, de-escalate, soften language, and redirect to constructive support."
                )
                cons_str = ", ".join(constraints) if constraints else "safety best-practices"
                safe_messages = [
                    {"role": "system", "content": guidance},
                    {
                        "role": "user",
                        "content": (
                            "The user is distressed and expressed harmful intent. "
                            f"Please respond safely and constructively, following these constraints: {cons_str}."
                        ),
                    },
                ]
                try:
                    assistant = chat(safe_messages)
                    print(f"🤖 {assistant}")
                    convo.append({"role": "assistant", "content": assistant})
                except Exception as e:
                    print(f"⚠ LLM fallback error: {e}")
                # Tail the ethics log to demonstrate local auditing
                root = mem.root()
                ethics_log = os.path.join(root, 'logbook', 'ethics.jsonl')
                tail = tail_file(ethics_log, 1)
                if tail:
                    print(f"   log: {tail[-1]}")
                continue
        except Exception as e:
            print(f"⚠ Precheck error (continuing): {e}")

        convo.append({"role": "user", "content": user})
        try:
            assistant = chat(convo)
        except Exception as e:
            print(f"❌ API error: {e}")
            print("Skipping this turn...")
            convo.pop()
            continue

        act = maybe_parse_action(assistant)
        reasoning_text = assistant
        if act:
            m = re.search(r'\{[^{}]*"action"[^{}]*\}', assistant, flags=re.DOTALL)
            if m:
                reasoning_text = assistant[:m.start()].strip()

        if reasoning_text:
            print(f"🤖 {reasoning_text}")

        if act:
            try:
                result = route(act, mem)
                action_name = act.get('action', 'unknown')
                print(f"\n🔧 Action: {action_name}")
                if result.get("ok"):
                    print("✅ Success")
                    if "memory_id" in result:
                        print(f"   💾 Stored as: {result['memory_id'][:30]}...")
                    if "reflection" in result and result["reflection"]:
                        print(f"   🤔 Reflection: {result['reflection']}")
                    if "stats" in result:
                        s = result["stats"]
                        print(f"   📊 Total: {s.get('total', 0)} | Archived: {s.get('archived', 0)}")
                        if s.get("by_lobe"):
                            lobe_info = ", ".join([f"{l}({c})" for l, c in s["by_lobe"][:3]])
                            print(f"   📚 By lobe: {lobe_info}")
                    if "recent_ids" in result:
                        ids = result["recent_ids"]
                        print(f"   📋 Found {len(ids)} recent memories")
                        for i, mid in enumerate(ids[:3]):
                            print(f"      {i+1}. {mid[:25]}...")
                    if "recall" in result:
                        r = result["recall"]
                        if isinstance(r, dict) and r.get("content"):
                            content = r["content"]
                            preview = content[:100] + "..." if len(content) > 100 else content
                            print(f"   📄 Content: {preview}")
                            print(f"   🗄️ Source: {r.get('source','auto')}")
                        else:
                            print("   ❌ Memory not found")
                    if "precheck_result" in result:
                        pre = result["precheck_result"]
                        decision = pre.get("decision", "unknown")
                        risk = pre.get("risk", "unknown")
                        icon = {"allow":"✅","allow_with_constraints":"⚠️","block":"🚫"}.get(decision,"❓")
                        print(f"   🛡️ Ethics: {icon} {decision.upper()} | Risk: {risk}")
                else:
                    print(f"❌ Failed: {result.get('error', 'Unknown error')}")

                convo.append({"role": "assistant", "content": assistant})
                if action_name == "recent" and result.get("ok") and result.get("recent_ids"):
                    recent_ids = result['recent_ids'][:3]
                    raw_texts: list[str] = []
                    details: list[str] = []
                    for mid in recent_ids:
                        r = mem.recall(mid)
                        if isinstance(r, dict) and r.get("content"):
                            t = r['content']
                            raw_texts.append(t)
                            details.append(f"Memory {mid[:12]}: {t[:200]} (src={r.get('source','auto')})")
                        else:
                            details.append(f"Memory {mid[:12]}: (not found)")
                    if raw_texts:
                        previews = []
                        for t in raw_texts[:3]:
                            t = (t or "").strip().replace('\n', ' ')
                            if len(t) > 80:
                                t = t[:80] + "..."
                            previews.append(t)
                        summary = "I remember: " + "; ".join(previews)
                        print(f"🤖 {summary}")
                        convo.append({"role": "assistant", "content": summary})
                    convo.append({"role": "user", "content": "[Recent memories retrieved:\n"+"\n".join(details)+"]"})
            except Exception as e:
                print(f"❌ Tool error: {e}")
                convo.append({"role": "assistant", "content": reasoning_text})
        else:
            convo.append({"role": "assistant", "content": assistant})

        if len(convo) > 20:
            convo = [convo[0]] + convo[-18:]


if __name__ == "__main__":
    run_repl()
