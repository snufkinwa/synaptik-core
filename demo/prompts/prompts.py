from pathlib import Path

# Load modular prompt fragments from this directory
PROMPTS_DIR = Path(__file__).parent

def load_prompt() -> str:
    """Join modular prompt files into one system prompt string."""
    order = [
        "system_header.md",
        "startup.md",
        "memory.md",
        "schema.toml",
        "routing.toml",
        "safety.toml",
        "examples.md",
    ]
    parts = []
    for filename in order:
        path = PROMPTS_DIR / filename
        if path.exists():
            text = path.read_text().strip()
            parts.append(f"## {filename.upper()}\n{text}")
    return "\n\n---\n\n".join(parts)

# Materialized system prompt for importers
SYSTEM_PROMPT = load_prompt()

def system_prompt() -> str:
    """Return the composed system prompt string.

    Mirrors the old API shape so callers can use either a variable
    or a function to fetch the prompt.
    """
    return SYSTEM_PROMPT
