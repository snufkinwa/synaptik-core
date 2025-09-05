from typing import Optional
from rich.console import Console
from rich.markdown import Markdown

_console: Optional[Console] = Console()


def print_assistant(text: str) -> None:
    """Render assistant output with Markdown support.

    Prints the robot on its own line, then renders the message as Markdown so
    headings, lists, and code blocks display correctly.
    """
    msg = text or ""
    if _console is not None:
        # Put a separate line so Markdown can render cleanly
        _console.print("\n")
        try:
            _console.print(Markdown(msg))
        except Exception:
            # Fallback to plain printing if Markdown rendering fails
            _console.print(msg)

