# demo/MVP_groq_responses.py
import os, json, re, time
from pathlib import Path
from dotenv import load_dotenv
from openai import OpenAI
from synaptik_core import PyCommands

# load ../.env 
load_dotenv(Path(__file__).resolve().parents[1] / ".env")

# --- Groq Responses API config (OpenAI client, Groq backend) ---
MODEL = os.environ.get("GROQ_MODEL", "openai/gpt-oss-20b")  
client = OpenAI(
    api_key=os.environ.get("GROQ_API_KEY"),
    base_url="https://api.groq.com/openai/v1",
)

# Synaptik Core bridge
cmd = PyCommands()

def recall_obj(mem_id: str, prefer: str | None = None):
    try:
        return cmd.recall(mem_id, prefer)
    except Exception:
        return None

def recall_text(mem_id: str, prefer: str | None = None):
    r = recall_obj(mem_id, prefer)
    if isinstance(r, dict):
        return r.get("content")
    return None

def recall_source(mem_id: str, prefer: str | None = None):
    r = recall_obj(mem_id, prefer)
    if isinstance(r, dict):
        return r.get("source")
    return None

SYSTEM = """You are the Synaptik Agent - a helpful assistant with persistent memory capabilities.

MEMORY BEHAVIOR:
- ALWAYS store important user information (preferences, profile, decisions, solutions) immediately when shared
- When users introduce themselves or share personal details, store in "preferences" lobe
- When users ask about problems or you provide solutions, store in "solutions" lobe  
- When users ask to remember something, store it immediately
- When users ask "what do you remember" or similar, use "recent" action to check memories
- If you see "[Previous context: ...]" messages, use that information to understand the user

STARTUP CONTEXT:
- When a conversation starts, you may receive previous context from stored memories
- Use this context to personalize your responses and remember the user
- Don't mention that you "just loaded" memories - act as if you remember naturally

RESPONSE FORMAT:
- Give a helpful response FIRST
- Then add the JSON action on the LAST line (if needed)
- Only ONE action per response

LOBES:
- "preferences": user profile, likes/dislikes, personal info
- "solutions": problems discussed, solutions provided, decisions made
- "chat": general conversation context worth keeping
- "insights": patterns, principles, important realizations

ACTIONS (put on last line as single JSON object):
{"action":"remember","args":{"lobe":"preferences","content":"descriptive summary","key":"optional_key"}}
{"action":"recent","args":{"lobe":"preferences","n":10}}
{"action":"recall","args":{"memory_id":"specific_id"}}
{"action":"stats","args":{"lobe":null}}
{"action":"reflect","args":{"lobe":"chat","window":50}}
{"action":"precheck","args":{"text":"content to check","purpose":"memory_storage"}}

EXAMPLES:

User: Hi! I'm Sarah, a software engineer working on AI safety.
Assistant: Nice to meet you, Sarah! I'll remember that you're a software engineer focused on AI safety.
{"action":"remember","args":{"lobe":"preferences","content":"User is Sarah, software engineer working on AI safety research","key":"user_profile"}}

User: What do you remember about me?
Assistant: Let me check what I have stored about you.
{"action":"recent","args":{"lobe":"preferences","n":10}}

User: My favorite color is blue.
Assistant: Got it - I'll remember that your favorite color is blue.
{"action":"remember","args":{"lobe":"preferences","content":"User's favorite color is blue","key":"favorite_color"}}

IMPORTANT: Always store user introductions, preferences, and important context immediately!"""

def maybe_parse_action(text: str):
    lines = text.strip().split('\n')
    last_line = lines[-1].strip()
    
    # Check if last line is JSON
    if last_line.startswith("{") and last_line.endswith("}"):
        try:
            return json.loads(last_line)
        except Exception:
            pass
    
    # Fallback to regex search
    m = re.search(r'\{[^{}]*"action"[^{}]*\}', text, flags=re.DOTALL)
    if m:
        try:
            return json.loads(m.group(0))
        except Exception:
            return None
    return None

def tool_router(action):
    name = (action or {}).get("action")
    args = (action or {}).get("args", {}) or {}
    
    if name == "remember":
        return {"ok": True, "memory_id": cmd.remember(
            args.get("lobe","chat"),
            args.get("content",""),
            args.get("key"),
        )}
    
    if name == "reflect":
        return {"ok": True, "reflection": cmd.reflect(
            args.get("lobe","chat"),
            int(args.get("window",20)),
        )}
    
    if name == "stats":
        return {"ok": True, "stats": cmd.stats(args.get("lobe"))}
    
    if name == "precheck":
        return {"ok": True, "precheck_result": cmd.precheck_text(
            args.get("text", ""),
            args.get("purpose", "general")
        )}
    
    if name == "recent":
        return {"ok": True, "recent_ids": cmd.recent(
            args.get("lobe", "chat"),
            int(args.get("n", 10))
        )}
    
    if name == "recall":
        memory_id = args.get("memory_id", "")
        if not memory_id:
            return {"ok": False, "error": "memory_id required"}
        prefer = args.get("prefer")
        result = cmd.recall(memory_id, prefer)
        return {"ok": True, "recall": result}
    
    return {"ok": False, "error": f"unknown action: {name}"}

def chat_with_responses_api(messages, retries=2, backoff=0.6):
    """
    Call Groq Responses API with proper parameters based on the documentation.
    """
    # Convert messages to a single input string for Responses API
    input_text = ""
    for msg in messages:
        if msg["role"] == "system":
            input_text += f"{msg['content']}\n\n"
        elif msg["role"] == "user":
            input_text += f"User: {msg['content']}\n\n"
        elif msg["role"] == "assistant":
            input_text += f"Assistant: {msg['content']}\n\n"
    
    for attempt in range(retries + 1):
        try:
            response = client.responses.create(
                model=MODEL,
                input=input_text.strip(),
                temperature=0.3,
                max_output_tokens=1500,
                reasoning={
                    "effort": "medium"
                }
            )
            return response.output_text or ""
        except Exception as e:
            error_msg = str(e)
            print(f"Responses API attempt {attempt + 1} failed: {error_msg}")
            
            if attempt < retries:
                if any(code in error_msg for code in ["500", "502", "503", "504", "Internal Server Error"]):
                    print(f"Retrying in {backoff * (attempt + 1)} seconds...")
                    time.sleep(backoff * (attempt + 1))
                    continue
                elif "rate limit" in error_msg.lower():
                    print(f"Rate limited, waiting {backoff * 2} seconds...")
                    time.sleep(backoff * 2)
                    continue
            
            raise Exception(f"Groq Responses API error after {attempt + 1} attempts: {error_msg}")

def chat_with_regular_api(messages, retries=2, backoff=0.6):
    """
    Fallback to regular chat completions API if Responses API fails.
    """
    for attempt in range(retries + 1):
        try:
            response = client.chat.completions.create(
                model=MODEL,
                messages=messages,
                temperature=0.3,
                max_tokens=1500,
            )
            return response.choices[0].message.content or ""
        except Exception as e:
            error_msg = str(e)
            print(f"Chat API attempt {attempt + 1} failed: {error_msg}")
            
            if attempt < retries:
                if any(code in error_msg for code in ["500", "502", "503", "504", "Internal Server Error"]):
                    time.sleep(backoff * (attempt + 1))
                    continue
                elif "rate limit" in error_msg.lower():
                    time.sleep(backoff * 2)
                    continue
            raise

def chat(messages, retries=2, backoff=0.6):
    """
    Try Responses API first, fallback to regular chat API if needed.
    """
    try:
        return chat_with_responses_api(messages, retries, backoff)
    except Exception as e:
        print(f"Responses API failed: {e}")
        print("Falling back to regular chat API...")
        return chat_with_regular_api(messages, retries, backoff)

def run_repl():
    print("🧠 Synaptik Agent x Groq Responses API — Persistent Memory & Ethics")
    print(f"🤖 Model: {MODEL}")
    print(f"💾 Root: {cmd.root()}")
    print()
    print("💡 This agent will:")
    print("   • Remember important information from our conversations")
    print("   • Build knowledge over time using Memory IDs")  
    print("   • Check ethics before storing sensitive content")
    print("   • Reference previous conversations using Memory IDs")
    print()
    print("✨ Try: 'Hi I'm [name], I like [thing]' or 'What do you remember about me?'")
    print("📝 Type 'exit' to quit")
    print("=" * 60)

    print("Type ':demo' anytime to run a quick end-to-end demo.")

    # Quick connectivity tests
    try:
        print("\n🧪 Testing APIs...")
        
        # Test Synaptik Core
        stats = cmd.stats(None)
        print(f"✓ Synaptik Core: {stats['total']} memories")
        
        # Test Groq API
        test_response = client.responses.create(
            model=MODEL,
            input="Hello! Respond with 'Test OK'",
            max_output_tokens=20
        )
        print(f"✓ Groq API: {test_response.output_text[:20]}...")
        
    except Exception as e:
        print(f"⚠ Startup test failed: {e}")

    # Initialize conversation
    convo = [{"role": "system", "content": SYSTEM}]

    # Load recent memories at startup to give context
    try:
        print("\n🧠 Loading recent memories...")
        recent_result = cmd.recent("preferences", 3)
        if recent_result:
            startup_memories = []
            for mem_id in recent_result[:3]:
                try:
                    r = cmd.recall(mem_id)
                    if isinstance(r, dict) and r.get("content"):
                        startup_memories.append(r["content"][:200])
                except:
                    pass
            
            if startup_memories:
                print("📚 Context from previous sessions:")
                for i, memory in enumerate(startup_memories):
                    preview = memory[:80] + "..." if len(memory) > 80 else memory
                    print(f"   {i+1}. {preview}")
                
                # Add context to conversation 
                context_summary = "Previous context from stored memories: " + "; ".join(startup_memories)
                convo.append({"role": "user", "content": f"[{context_summary}]"})
                convo.append({"role": "assistant", "content": "I can see our previous conversation context. How can I help you today?"})
            else:
                print("📝 No previous context found - starting fresh!")
        
    except Exception as e:
        print(f"⚠ Memory loading failed: {e}")
        print("📝 Starting fresh!")

    print("\n" + "=" * 60)
    
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

        # Scripted demo trigger
        if user.strip().lower() in {":demo", ":d"}:
            try:
                run_demo()
            except Exception as e:
                print(f"❌ Demo error: {e}")
            continue

        convo.append({"role": "user", "content": user})
        
        try:
            assistant = chat(convo)
        except Exception as e:
            print(f"❌ API error: {e}")
            print("Skipping this turn...")
            convo.pop()
            continue

        # Parse action from response
        act = maybe_parse_action(assistant)

        # Extract reasoning text (everything before JSON)
        reasoning_text = assistant
        if act:
            # Remove JSON from the response text
            json_match = re.search(r'\{[^{}]*"action"[^{}]*\}', assistant, flags=re.DOTALL)
            if json_match:
                reasoning_text = assistant[:json_match.start()].strip()

        # Show assistant's response
        if reasoning_text:
            print(f"🤖 {reasoning_text}")

        # Execute action if present
        if act:
            try:
                result = tool_router(act)
                action_name = act.get('action', 'unknown')
                print(f"\n🔧 Action: {action_name}")
                
                if result.get("ok"):
                    print("✅ Success")

                    if "memory_id" in result:
                        mem_id = result["memory_id"]
                        print(f"   💾 Stored as: {mem_id[:30]}...")

                    if "reflection" in result and result["reflection"]:
                        print(f"   🤔 Reflection: {result['reflection']}")

                    if "stats" in result:
                        stats = result["stats"]
                        print(f"   📊 Total: {stats.get('total', 0)} | Archived: {stats.get('archived', 0)}")
                        if stats.get("by_lobe"):
                            lobe_info = ", ".join([f"{lobe}({count})" for lobe, count in stats["by_lobe"][:3]])
                            print(f"   📚 By lobe: {lobe_info}")

                    if "recent_ids" in result:
                        ids = result["recent_ids"]
                        print(f"   📋 Found {len(ids)} recent memories")
                        for i, mem_id in enumerate(ids[:3]):
                            print(f"      {i+1}. {mem_id[:25]}...")

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
                        precheck = result["precheck_result"]
                        decision = precheck.get("decision", "unknown")
                        risk = precheck.get("risk", "unknown")
                        decision_icon = {"allow":"✅","allow_with_constraints":"⚠️","block":"🚫"}.get(decision,"❓")
                        print(f"   🛡️ Ethics: {decision_icon} {decision.upper()} | Risk: {risk}")

                else:
                    print(f"❌ Failed: {result.get('error', 'Unknown error')}")

                # Add to conversation history
                convo.append({"role": "assistant", "content": assistant})
                
                # If it was a "recent" action, automatically recall the actual content
                if action_name == "recent" and result.get("ok") and result.get("recent_ids"):
                    recent_ids = result['recent_ids'][:3]  # Get top 3 most recent
                    memories_content = []
                    raw_texts = []
                    
                    for mem_id in recent_ids:
                        try:
                            r = cmd.recall(mem_id)
                            if isinstance(r, dict) and r.get("content"):
                                text = r['content']
                                raw_texts.append(text)
                                memories_content.append(f"Memory {mem_id[:12]}: {text[:200]} (src={r.get('source','auto')})")
                            else:
                                memories_content.append(f"Memory {mem_id[:12]}: (not found)")
                        except Exception as e:
                            memories_content.append(f"Memory {mem_id[:12]}: (error: {e})")
                    
                    memory_summary = "\n".join(memories_content)
                    # Produce a concise natural summary for the user
                    if raw_texts:
                        previews = []
                        for t in raw_texts[:3]:
                            t = (t or "").strip().replace('\n', ' ')
                            if len(t) > 80:
                                t = t[:80] + "..."
                            previews.append(t)
                        human_summary = "I remember: " + "; ".join(previews)
                        print(f"🤖 {human_summary}")
                        convo.append({"role": "assistant", "content": human_summary})

                    # Feed precise details back as hidden context for the LLM
                    convo.append({"role": "user", "content": f"[Recent memories retrieved:\n{memory_summary}]"})
                else:
                    convo.append({"role": "user", "content": f"[Action completed: {action_name}]"})
                    
            except Exception as e:
                print(f"❌ Tool error: {e}")
                convo.append({"role": "assistant", "content": reasoning_text})
        else:
            convo.append({"role": "assistant", "content": assistant})

        # Keep conversation manageable
        if len(convo) > 20:
            convo = [convo[0]] + convo[-18:]


def tail_file(path: str, n: int = 3) -> list[str]:
    try:
        with open(path, 'r') as f:
            lines = f.readlines()
        return [ln.rstrip('\n') for ln in lines[-n:]]
    except Exception:
        return []


def run_demo():
    print("\n🚀 Running scripted demo...")
    root = cmd.root()
    print(f"   Root: {root}")

    # 1) Persist a preference
    pref_text = "User prefers short, friendly greetings"
    pref_id = cmd.remember("preferences", pref_text, "user_pref")
    print(f"   💾 Saved preference id: {pref_id[:24]}...")

    # 2) Show that chat lobe auto-promotes once it reaches 5 hot rows
    chat_stats_before = cmd.stats("chat")
    before_total = chat_stats_before.get('total', 0)
    before_arch = chat_stats_before.get('archived', 0)
    before_hot = max(0, before_total - before_arch)
    need = max(0, 5 - before_hot)
    for i in range(need):
        cmd.remember("chat", f"demo chat note {i+1}", None)
    chat_stats_after = cmd.stats("chat")
    print(f"   📊 Chat before: total={before_total}, archived={before_arch}")
    print(f"   📊 Chat after:  total={chat_stats_after.get('total',0)}, archived={chat_stats_after.get('archived',0)}")

    # 3) Pick a recent chat memory and show its recall source
    chat_ids = cmd.recent("chat", 1)
    if chat_ids:
        rid = chat_ids[0]
        r = cmd.recall(rid, "auto")
        if isinstance(r, dict):
            src = r.get('source', 'auto')
            prev = (r.get('content') or '')[:80]
            print(f"   🔎 Recall {rid[:18]}... source={src}, content='{prev}'")

    # 4) Lobe separation: preference vs solution
    sol_id = cmd.remember("solutions", "Final answer: 42 because constraints...", "solution_1")
    pref_recent = (cmd.recent("preferences", 1) or [None])[0]
    sol_recent = (cmd.recent("solutions", 1) or [None])[0]
    if pref_recent:
        rp = cmd.recall(pref_recent, "auto")
        if isinstance(rp, dict):
            print(f"   📁 preferences → {rp.get('content','')[:60]}")
    if sol_recent:
        rs = cmd.recall(sol_recent, "auto")
        if isinstance(rs, dict):
            print(f"   📁 solutions   → {rs.get('content','')[:60]}")

    # 5) Ethics precheck and audit tail
    res = cmd.precheck_text("I want to kill her", "chat_message")
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

if __name__ == "__main__":
    run_repl()
