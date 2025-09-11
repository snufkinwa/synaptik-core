"""
Synaptik Core Groq Chat Demo (REPL)

Usage:
  - Ensure the Python bindings are built via maturin and dependencies installed.
  - Put your Groq key in a .env at the repo root (GROQ_API_KEY, GROQ_MODEL).
  - Run: `python -m demo.demo`

Tips:
  - Type `:demo` to run a quick scripted end-to-end flow.
  - To replicate the demo video, open `demo/test_prompts_syn.txt` and paste
    the prompts phase-by-phase (Phase 1 → 13) into the REPL.
"""

# Standard library
import sys
from pathlib import Path
import re  # drop if unused
from typing import Dict, List  # drop if unused

# Add this file's directory to sys.path (script mode only)
sys.path.append(str(Path(__file__).resolve().parent))

# Local imports
from ui import print_assistant
from flows import run_demo_flow
from intents import (
    handle_action_plan,
    handle_audit_trail,
    handle_branch_hop,
    handle_branch_steps,
    handle_recall_sources_query,
    handle_recent_query,
    handle_root_query,
    handle_trace_path_cmd,
    handle_recall_latest_path,
    handle_source_test,
    looks_like_action_plan,
    looks_like_audit_trail,
    looks_like_branch_hop,
    looks_like_branch_steps,
    looks_like_recall_sources_query,
    looks_like_recent_query,
    looks_like_root_query,
    looks_like_trace_path_cmd,
    looks_like_recall_latest_path,
    looks_like_source_test,
)
from llm_client import MODEL, chat
from actions import maybe_parse_action, route
from memory_bridge import MemoryBridge
from prompts.prompts import system_prompt

# Precompiled regexes for quick temperature heuristics
FACT_RE = re.compile(r"\b(what|when|where|who|which|define|explain|fact|facts|cite|citation|exact)\b", re.I)
CREATIVE_RE = re.compile(r"\b(plan|brainstorm|idea|ideas|design|rewrite|draft|story|creative|solve|solution|approach|explore|refactor|migrate|replace|rebuild|clean\s*slate|start\s*fresh)\b", re.I)


def choose_temperature(user_text: str) -> float:
    t = (user_text or "").lower()
    # Low temperature for factual queries or source/recent introspection
    if looks_like_source_test(t) or looks_like_recent_query(t):
        return 0.25
    if FACT_RE.search(t):
        return 0.25
    # Higher temperature for creative/solution-oriented work
    if CREATIVE_RE.search(t):
        return 0.65
    # Default uses baseline inside llm_client
    return 0.5



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
    print("Tip: To replicate the demo video, paste prompts from demo/test_prompts_syn.txt (Phase 1 → 13).")
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
            # Batch recall to reduce round-trips into the binding
            results = mem.recall_many(recent_ids)
            for r in results:
                if isinstance(r, dict) and r.get("content"):
                    text = (r["content"] or "")[:200]
                    if text:
                        startup_memories.append(text)
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
                    # Lower temperature for greeting to reduce odd phrasing
                    assistant = chat(convo, temperature=0.35)
                    act = maybe_parse_action(assistant)
                    reasoning_text = assistant
                    if act:
                        m = re.search(r"\{[^{}]*\"action\"[^{}]*\}", assistant, flags=re.DOTALL)
                        if m:
                            reasoning_text = assistant[:m.start()].strip()
                    if reasoning_text:
                        print_assistant(reasoning_text)
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

        # Run local precheck BEFORE hitting the LLM (ensures audit/ethics logs record)
        llm_temp_hint = None  # may lower temperature for constrained replies
        try:
            pre = mem.cmd.precheck_text(user, "chat_message")
            decision = pre.get("decision", "allow")
            risk = pre.get("risk", "?")
            icon = {"allow":"✅","allow_with_constraints":"⚠️","block":"🚫"}.get(decision,"❓")
            print(f"🛡️ Precheck: {icon} {decision.upper()} | Risk: {risk}")
            constraints = pre.get("constraints") or []
            # If soft-allowed, pass constraints directly to the LLM as a minimal system message (no policy hardcoding)
            if decision == "allow_with_constraints":
                # Provide contract-derived constraints and explicitly instruct use of the CONSTRAINTS TEMPLATE (no refusal).
                cons_str = ", ".join(constraints) if constraints else ""
                convo.append({
                    "role": "system",
                    "content": (
                        "Contracts decision=ALLOW_WITH_CONSTRAINTS; "
                        f"risk={risk}; reason={pre.get('reason','')}; "
                        f"constraints=[{cons_str}]. "
                        "Use the CONSTRAINTS TEMPLATE from your system instructions. Do not refuse; do not use the refusal template. "
                        "If the user proposes ignoring safety to go faster, offer a safer plan that preserves safety while improving speed. "
                        "Reply directly with practical steps. Do not mention or describe constraints, policy, risk, or safety. "
                        "Do not label your reply as an example."
                    )
                })
                # Special-case: benign idioms like "this is killing me" / tech usage of kill
                idiom_flags = {"clarify_nonliteral_intent", "acknowledge_frustration", "do_not_refuse"}
                tech_flags = {"avoid_encouraging_destruction"}
                if (set(constraints) & idiom_flags) or (set(constraints) & tech_flags):
                    convo.append({
                        "role": "system",
                        "content": (
                            "Treat any 'kill/killing' phrasing here as benign idiom or technical slang (e.g., kill the bug/process). "
                            "Do not provide crisis or self-harm intervention content. "
                            "Respond with empathetic but practical developer help: clarify the issue briefly, and offer step-by-step debugging/IDE tips."
                        )
                    })
                llm_temp_hint = 0.35
            # If blocked, do not call the LLM. Use strict refusal template.
            if decision == "block":
                refusal = "I can’t assist with that."
                print_assistant(refusal)
                convo.append({"role": "assistant", "content": refusal})
                continue
        except Exception as e:
            print(f"⚠ Precheck error (continuing): {e}")

        # Heuristic intent: "show recent memories" with sources — handle locally without LLM
        if looks_like_recent_query(user):
            try:
                handle_recent_query(mem, user, convo)
            except Exception as e:
                print(f"❌ Recent fetch error: {e}")
            continue

        # Branch/replay helpers by plain language (no JSON)
        if looks_like_branch_hop(user):
            try:
                handle_branch_hop(mem, user)
            except Exception as e:
                print(f"❌ Branch hop error: {e}")
            continue

        if looks_like_branch_steps(user):
            try:
                handle_branch_steps(mem, user)
            except Exception as e:
                print(f"❌ Branch steps error: {e}")
            continue

        if looks_like_recall_latest_path(user):
            try:
                handle_recall_latest_path(mem, user)
            except Exception as e:
                print(f"❌ Recall-latest error: {e}")
            continue

        if looks_like_trace_path_cmd(user):
            try:
                handle_trace_path_cmd(mem, user)
            except Exception as e:
                print(f"❌ Trace-path error: {e}")
            continue

        if looks_like_source_test(user):
            try:
                handle_source_test(mem, user)
            except Exception as e:
                print(f"❌ Source test error: {e}")
            continue

        # Audit trail request — handled locally (no LLM)
        if looks_like_audit_trail(user):
            try:
                handle_audit_trail(mem)
            except Exception as e:
                print(f"❌ Audit trail error: {e}")
            continue

        # If the user asks for a personalized action plan, prepare context then fall through to LLM
        if looks_like_action_plan(user):
            try:
                handle_action_plan(mem, user, convo)
            except Exception as e:
                print(f"❌ Planning context error: {e}")

        # Pure recall/source queries should not invoke LLM
        elif looks_like_recall_sources_query(user):
            try:
                handle_recall_sources_query(mem, user)
            except Exception as e:
                print(f"❌ Recall error: {e}")
            continue

        if looks_like_root_query(user):
            try:
                handle_root_query(mem)
            except Exception as e:
                print(f"❌ Root/persistence error: {e}")
            continue

        convo.append({"role": "user", "content": user})
        try:
            temp = choose_temperature(user)
            if llm_temp_hint is not None:
                temp = min(temp, llm_temp_hint)
            assistant = chat(convo, temperature=temp)
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
            print_assistant(reasoning_text)

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
                        print_assistant(summary)
                        convo.append({"role": "assistant", "content": summary})
                    convo.append({"role": "user", "content": "[Recent memories retrieved:\n"+"\n".join(details)+"]"})
            except Exception as e:
                print(f"❌ Tool error: {e}")
                convo.append({"role": "assistant", "content": reasoning_text})
        else:
            convo.append({"role": "assistant", "content": assistant})

        if len(convo) > 12:
            convo = [convo[0]] + convo[-10:]


if __name__ == "__main__":
    run_repl()
