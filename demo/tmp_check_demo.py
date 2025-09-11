import sys
from pathlib import Path
sys.path.append(str(Path(__file__).resolve().parent))

from memory_bridge import MemoryBridge
from flows import run_demo_flow

if __name__ == "__main__":
    mem = MemoryBridge()
    run_demo_flow(mem)

