# 🧠 synaptik-core (WIP)

**Toward Trustworthy AGI — a lightweight cognitive architecture for ethical, agentic intelligence.**


## What is synaptik-core?

`synaptik-core` is an experimental cognitive framework that combines **LLMs**, **symbolic reasoning**, and **moral contract enforcement** to create agents that are:

- Ethically aligned  
-  Memory-capable (stateful)  
- ⚡ Edge-deployable  
-  Auditable and interpretable

This project addresses key limitations in modern AI: statelessness, hallucination, lack of transparency, and ethical ambiguity.

##  Architecture Overview

The system is built on three core layers:

| Component          | Description                                                                 |
|--------------------|-----------------------------------------------------------------------------|
| **DAG Memory**     | A Directed Acyclic Graph for long-term, symbolic memory and planning.       |
| **SQLite Cache**   | A synthetic hippocampus for fast recall of recent interactions.             |
| **Moral Contracts**| Declarative, enforceable ethical rules evaluated by a dedicated agent.      |

These are managed by modular, collaborative agents:

- **Ethos Agent** – Evaluates prompts and actions against moral contracts  
- **Librarian Agent** – Retrieves and indexes memory  
- **Memory Agent** – Handles caching, pruning, and DAG updates  
- **Execution Agent** – Generates responses and takes actions  
- **Audit Agent** – Logs key decisions to an immutable ledger or cold storage  


## Use Cases

- AI safety research  
- Explainable AI (XAI) systems  
- Cognitive agents for sensitive domains (health, education, law)  
- Memory-augmented LLMs for edge devices  

## Tech Stack

| Tech                | Role                                                                                                                         |
| ------------------- | ---------------------------------------------------------------------------------------------------------------------------- |
| **Rust / Python**   | Core logic implementation (currently WIP in Rust; Python used for prototypes and orchestration)                              |
| **SQLite**          | Fast-access working memory for recent or frequently used information                                                         |
| **IPFS**            | Cold memory archiving. Used to offload large or infrequently accessed nodes and audit logs to a decentralized storage layer. |
| **DAG**             | Immutable memory graph. Planned as a tamper-proof audit trail of agent decisions, ethical evaluations, and memory updates.   |
| **OpenAI** | LLM-based reasoning and generation. Handles reflection, inference, and dialog components within the agentic system.          |


## 🚧 Project Status

`synaptik-core` is in **early development**. The current prototype demonstrates:

- The current prototype demonstrates a limited DAG-based symbolic memory that supports adding, retrieving, and pruning memory nodes



## License

MIT License. Use it, fork it, remix it — just cite the project and respect the mission.


## Author

**Janay Harris**  
Independent AI Architect & Researcher | Cloud Developer  
[LinkedIn](https://www.linkedin.com/in/janay-codes/) | janayharris@synaptik-core.dev


## Citation

If you're referencing the ideas or architecture in academic work:

> Harris, J. (2025). *synaptik-core: Toward Trustworthy AGI via Hybrid Cognitive Architecture*. ColorStack Summit 2025.



##  Vision

This is a step toward cognitive agents that can **remember**, **reason**, and **act with integrity** even in ambiguous, real-world contexts.

