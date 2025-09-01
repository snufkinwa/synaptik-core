# demo/MVP_groq_responses.py
import os, json, re, time
from pathlib import Path
from dotenv import load_dotenv
from openai import OpenAI
from synaptik_core import PyCommands

# load ../.env 
load_dotenv(Path(__file__).resolve().parents[1] / ".env")

# --- Groq Responses API config (OpenAI client, Groq backend) ---
MODEL = os.environ.get("GROQ_MODEL", "openai/gpt-oss-20b")  # Use the correct OpenAI model
client = OpenAI(
    api_key=os.environ.get("GROQ_API_KEY"),
    base_url="https://api.groq.com/openai/v1",
)

# Synaptik Core bridge
cmd = PyCommands()

SYSTEM = """You are the Synaptik Agent.
- Synaptik Core handles persistence/reflection/stats. You are stateless.
- When you need an action, emit ONE JSON object line:
  {"action":"remember","args":{"lobe":"chat","content":"...","key":null}}
  {"action":"reflect","args":{"lobe":"chat","window":50}}
  {"action":"stats","args":{"lobe":null}}
- Otherwise, reply in plain text.
"""

def maybe_parse_action(text: str):
    t = text.strip()
    if t.startswith("{") and t.endswith("}"):
        try:
            return json.loads(t)
        except Exception:
            pass
    m = re.search(r"\{.*?\}", text, flags=re.DOTALL)  # non-greedy
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
            args.get("lobe","notes"),
            args.get("content",""),
            args.get("key"),
        )}
    if name == "reflect":
        return {"ok": True, "reflection": cmd.reflect(
            args.get("lobe","notes"),
            int(args.get("window",20)),
        )}
    if name == "stats":
        return {"ok": True, "stats": cmd.stats(args.get("lobe"))}
    return {"ok": False, "error": f"unknown action: {name}"}

def chat_with_responses_api(messages, retries=2, backoff=0.6):
    """
    Call Groq Responses API with proper parameters based on the documentation.
    The Responses API expects a simple string input, not a messages array.
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
    
    # Don't add "Assistant:" at the end - let the model generate naturally
    
    for attempt in range(retries + 1):
        try:
            response = client.responses.create(
                model=MODEL,
                input=input_text.strip(),
                temperature=0.2,
                max_output_tokens=512,
                reasoning={
                    "effort": "medium"
                }
            )
            return response.output_text or ""
        except Exception as e:
            error_msg = str(e)
            print(f"Responses API attempt {attempt + 1} failed: {error_msg}")
            
            # Check for specific error types
            if attempt < retries:
                if any(code in error_msg for code in ["500", "502", "503", "504", "Internal Server Error"]):
                    print(f"Retrying in {backoff * (attempt + 1)} seconds...")
                    time.sleep(backoff * (attempt + 1))
                    continue
                elif "rate limit" in error_msg.lower():
                    print(f"Rate limited, waiting {backoff * 2} seconds...")
                    time.sleep(backoff * 2)
                    continue
            
            # If we've exhausted retries or it's a different error, raise
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
                temperature=0.2,
                max_tokens=512,
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
    print("Synaptik x Groq Responses API — type 'exit' to quit")
    print(f"Model: {MODEL}")
    print("root:", cmd.root())

    # Test API connectivity and validate model
    try:
        models = client.models.list()
        available_models = [m.id for m in models.data]
        if MODEL in available_models:
            print(f"✓ Model '{MODEL}' is available")
        else:
            print(f"⚠ Model '{MODEL}' not found in available models")
            # Show GPT-OSS models specifically
            gpt_oss_models = [m for m in available_models if "gpt-oss" in m]
            if gpt_oss_models:
                print(f"Available GPT-OSS models: {gpt_oss_models}")
            else:
                print("No GPT-OSS models found. Available models:")
                for model in available_models[:10]:  # Show first 10
                    print(f"  - {model}")
    except Exception as e:
        print(f"Warning: Could not list models: {e}")

    # Test a simple Responses API call
    try:
        print("\n🧪 Testing Responses API...")
        test_response = client.responses.create(
            model=MODEL,
            input="Hello! Please respond with just 'API test successful'.",
            max_output_tokens=50
        )
        print(f"✓ Responses API test: {test_response.output_text[:50]}...")
    except Exception as e:
        print(f"⚠ Responses API test failed: {e}")
        print("Will fall back to regular chat API")

    convo = [{"role": "system", "content": SYSTEM}]
    
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

        convo.append({"role": "user", "content": user})
        
        try:
            assistant = chat(convo)
        except Exception as e:
            print(f"❌ API error: {e}")
            print("Skipping this turn...")
            convo.pop()  # Remove the user message we just added
            continue

        # Check if the response contains an action
        act = maybe_parse_action(assistant)
        if act:
            try:
                result = tool_router(act)
                print(f"🔧 Action: {act.get('action', 'unknown')}")
                if result.get("ok"):
                    print("✅ Success")
                    # Show relevant data
                    if "memory_id" in result:
                        print(f"   Memory ID: {result['memory_id']}")
                    if "reflection" in result and result["reflection"]:
                        print(f"   Reflection: {result['reflection']}")
                    if "stats" in result:
                        stats = result["stats"]
                        print(f"   Total memories: {stats.get('total', 0)}")
                        if stats.get('by_lobe'):
                            print(f"   By lobe: {stats['by_lobe'][:3]}")  # Show top 3
                else:
                    print(f"❌ Failed: {result.get('error', 'Unknown error')}")
                
                # Feed tool result back for context
                convo.append({"role": "assistant", "content": assistant})
                convo.append({"role": "user", "content": f"[Tool result: {json.dumps(result)}]"})
                
            except Exception as e:
                print(f"❌ Tool execution error: {e}")
                convo.append({"role": "assistant", "content": assistant})
        else:
            print(f"🤖 {assistant}")
            convo.append({"role": "assistant", "content": assistant})

        # Keep conversation manageable
        if len(convo) > 20:
            # Keep system message and last 18 messages
            convo = [convo[0]] + convo[-18:]

if __name__ == "__main__":
    run_repl()