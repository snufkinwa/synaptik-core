from synaptik_core import PyCommands

cmd = PyCommands()
print("root:", cmd.root())

# mid = cmd.remember("chat", "short test content")  # key defaults internally
# print("memory_id:", mid)

# print("reflect:", cmd.reflect("chat", 20))
# print("stats:", cmd.stats(None))


blob = (
  "We’re building a Rust + Python agent. The agent uses a memory system in SQLite and a file archive. "
  "This agent demo focuses on trustworthy, auditable memories. The agent uses reflection to keep tags current. "
  "Our agent runs TF-IDF style reflection over summaries. "
)

# Make sure each is >500 chars and shares repeated keywords like 'agent', 'memory', 'reflection'
for i in range(3):
    text = (blob + f" Iteration {i}. ") * 6  # repeat to exceed 500 chars
    cmd.remember("chat", text)

print("reflect:", cmd.reflect("chat", 50))
print("stats:", cmd.stats(None))
