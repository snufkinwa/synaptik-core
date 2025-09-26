#!/usr/bin/env python3
"""Integration test ensuring large Python integers are not coerced through float and lose precision."""
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

    BIG = 10 ** 20  # exceeds IEEE754 exact integer range and i64::MAX
    U64_EDGE_PLUS_ONE = (2 ** 63)  # i64::MAX + 1 fits in u64

    with tempfile.TemporaryDirectory(prefix="synaptik-largeint-") as tmp:
        with chdir(tmp):
            c = sc.PyCommands()
            # Store large ints inside metadata for a snapshot path operation to pass through py_to_json.
            # Create a path by branching from a lobe. Use remember -> begin_branch to seed base.
            c.remember("chat", "seed memory")
            base_branch = c.begin_branch("chat", "pathA")
            assert isinstance(base_branch, str)
            sid = c.encode_engram("pathA", "content", {"big": BIG, "u64_edge_plus_one": U64_EDGE_PLUS_ONE})
            assert isinstance(sid, str)
            # Retrieve metadata via snapshot_meta and locate inserted values.
            meta_obj = c.snapshot_meta(sid)
            # meta_obj is a Python dict representation of JSON.
            # path meta structure: should contain our fields nested under 'meta' maybe depending on implementation.
            # We inserted directly as meta, so meta_obj should have keys we added OR have them within meta_obj itself.
            # Inspect recursively.
            def find_key(d, key):
                if isinstance(d, dict):
                    if key in d:
                        return d[key]
                    for v in d.values():
                        r = find_key(v, key)
                        if r is not None:
                            return r
                elif isinstance(d, list):
                    for v in d:
                        r = find_key(v, key)
                        if r is not None:
                            return r
                return None

            big_val = find_key(meta_obj, "big")
            u64_val = find_key(meta_obj, "u64_edge_plus_one")
            # BIG > u64::MAX, so expected representation choice (string fallback or Python int if JSON serialized as string then parsed?)
            # Our Rust logic currently stringifies only when falling back after attempts; BIG will not fit u64 and > i64 so becomes string via str() branch.
            assert big_val == str(BIG), f"Expected big int as precise string, got {big_val!r}"  # maintain precision
            assert u64_val == U64_EDGE_PLUS_ONE, f"Expected u64 boundary int exact, got {u64_val!r}"
    print("LARGE_INT_JSON OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
