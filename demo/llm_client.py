import os
import time
from pathlib import Path
from dotenv import load_dotenv
from openai import OpenAI
from typing import List, Dict


# Load ../.env once
load_dotenv(Path(__file__).resolve().parents[1] / ".env")

MODEL = os.environ.get("GROQ_MODEL", "openai/gpt-oss-20b")
client = OpenAI(
    api_key=os.environ.get("GROQ_API_KEY"),
    base_url="https://api.groq.com/openai/v1",
)


def chat_with_responses_api(messages: List[Dict[str, str]], retries: int = 2, backoff: float = 0.6) -> str:
    input_text = ""
    for msg in messages:
        role = msg.get("role")
        content = msg.get("content", "")
        if role == "system":
            input_text += f"{content}\n\n"
        elif role == "user":
            input_text += f"User: {content}\n\n"
        elif role == "assistant":
            input_text += f"Assistant: {content}\n\n"

    for attempt in range(retries + 1):
        try:
            resp = client.responses.create(
                model=MODEL,
                input=input_text.strip(),
                temperature=0.5,
                max_output_tokens=1500,
                reasoning={"effort": "medium"},
            )
            return resp.output_text or ""
        except Exception as e:
            msg = str(e)
            print(f"Responses API attempt {attempt + 1} failed: {msg}")
            if attempt < retries:
                if any(code in msg for code in ["500", "502", "503", "504", "Internal Server Error"]):
                    time.sleep(backoff * (attempt + 1))
                    continue
                if "rate limit" in msg.lower():
                    time.sleep(backoff * 2)
                    continue
            raise


def chat_with_regular_api(messages: List[Dict[str, str]], retries: int = 2, backoff: float = 0.6) -> str:
    for attempt in range(retries + 1):
        try:
            resp = client.chat.completions.create(
                model=MODEL,
                messages=messages,
                temperature=0.5,
                max_tokens=1500,
            )
            return resp.choices[0].message.content or ""
        except Exception as e:
            msg = str(e)
            print(f"Chat API attempt {attempt + 1} failed: {msg}")
            if attempt < retries:
                if any(code in msg for code in ["500", "502", "503", "504", "Internal Server Error"]):
                    time.sleep(backoff * (attempt + 1))
                    continue
                if "rate limit" in msg.lower():
                    time.sleep(backoff * 2)
                    continue
            raise


def chat(messages: List[Dict[str, str]], retries: int = 2, backoff: float = 0.6) -> str:
    try:
        return chat_with_responses_api(messages, retries, backoff)
    except Exception as e:
        print(f"Responses API failed: {e}")
        print("Falling back to regular chat API...")
        return chat_with_regular_api(messages, retries, backoff)
