#!/usr/bin/env python3
import json
import os
import sys
import tempfile
from contextlib import contextmanager


@contextmanager
def chdir(path: str):
    prev = os.getcwd()
    os.chdir(path)
    try:
        yield
    finally:
        os.chdir(prev)


def main() -> int:
    import synaptik_core as sc

    with tempfile.TemporaryDirectory(prefix="synaptik-smoke-") as tmp:
        with chdir(tmp):
            c = sc.PyCommands()
            root = c.root()
            print("root:", root)
            assert os.path.basename(root) == ".cogniv"

            res = c.govern_text("chat.reply", "Hello world!")
            print("govern_text:", res)
            assert res["status"] in {"ok", "violated", "stopped", "escalated"}

            mid = c.remember("chat", "hi there")
            print("remember id:", mid)
            rec = c.recall(mid)
            print("recall:", rec)
            assert rec is not None and rec.get("content") == "hi there"

            s = c.stats()
            print("stats:", json.dumps(s, indent=2))
            assert s.get("total", 0) >= 1

    print("SMOKE OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
