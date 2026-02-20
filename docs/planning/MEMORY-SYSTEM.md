# AgentOS Total Recall — Perfect Memory System
## The OS That Never Forgets

---

## 0. THE PROBLEM WITH OPENCLAW'S BUILT-IN MEMORY

OpenClaw's memory is designed for **one thing**: helping a chatbot remember
conversations. It stores markdown files, chunks them, embeds them in SQLite
via sqlite-vec, and retrieves with hybrid BM25 + vector search.

**That's not OS memory. That's chat history.**

An OS agent that truly controls the machine needs to remember:

- Every package you ever installed, when, why, what it broke
- Every system error, what caused it, how it was fixed
- Every configuration change across every NixOS generation
- Every file you created, moved, deleted — and the context around it
- Every service you ran, its performance characteristics, its failure modes
- Every network issue, DNS resolution, firewall change
- Every user command pattern — what you do at 9am vs 2am
- Every diagnosis it ever performed — so it never re-diagnoses the same thing
- Every approval/denial decision and why

**OpenClaw's sqlite-vec tops out at ~50K cached embeddings. It was designed
for daily notes, not continuous system telemetry.**

We need something fundamentally different.

---

## 1. ZVEC: WHY IT'S THE RIGHT ENGINE

### What Zvec Actually Is

Zvec is Alibaba's in-process vector database, built on **Proxima** — the same
engine that powers search across Alibaba's entire platform. Released Feb 2026.

**Why it's perfect for AgentOS:**

| Property | Zvec | OpenClaw's sqlite-vec |
|----------|------|-----------------------|
| Architecture | In-process C++ (Proxima) | SQLite extension |
| Scale | 10M+ vectors, 8500+ QPS | ~50K practical limit |
| Vector types | Dense + Sparse (multi-vector per doc) | Dense only |
| Hybrid search | Built-in scalar filters on index path | Separate BM25 + vector merge |
| Reranker | Built-in (weighted fusion, RRF) | Manual score fusion |
| Index | HNSW with int8/fp16 quantization | Basic HNSW |
| Persistence | Disk-backed, crash-safe | SQLite file |
| Overhead | ~zero (library, no daemon) | ~zero (SQLite extension) |
| Build time | 10M vectors in ~1 hour | Not designed for this scale |

**The killer feature for us: Zvec runs in-process.** No server. No daemon.
It's literally a library call. agentd embeds it directly — the vector DB
IS part of the kernel daemon. Zero deployment complexity. Zero network hops.

### What Zvec Gives Us That sqlite-vec Can't

1. **Multi-vector per document**: A single memory event can have BOTH a
   semantic embedding (what it means) AND a code/technical embedding (what
   it contains). Search with either or both simultaneously.

2. **Scalar filters pushed into the index**: "Find similar errors from the
   last 24 hours" doesn't scan all vectors then filter — it filters DURING
   the ANN search. This is critical for time-windowed recall.

3. **Built-in reranker**: Weighted fusion and RRF are native. No manual
   score normalization like OpenClaw's `bm25RankToScore(rank)` hack.

4. **Quantized indexes**: int8 quantization means 4x less memory per vector
   with minimal recall loss. For an OS storing millions of events, this
   matters.

5. **Dense + Sparse in one query**: Combine semantic similarity with
   keyword-exact matching in a single query call, not two separate searches
   merged after the fact.

---

## 2. ARCHITECTURE: THE THREE MEMORY TIERS

```
╔═══════════════════════════════════════════════════════════════════╗
║                     TIER 1: HOT MEMORY                          ║
║                     (in-process, <100ms)                        ║
║                                                                   ║
║  Zvec collection: "hot"                                          ║
║  Last 24 hours of events + current session context               ║
║  ~10K-50K vectors, fully in RAM                                  ║
║  Queried on EVERY user prompt automatically                      ║
║                                                                   ║
╠═══════════════════════════════════════════════════════════════════╣
║                     TIER 2: WARM MEMORY                         ║
║                     (on-disk index, <500ms)                     ║
║                                                                   ║
║  Zvec collection: "warm"                                         ║
║  Last 90 days of events, configurations, diagnoses               ║
║  ~100K-1M vectors, HNSW on disk with int8 quantization           ║
║  Queried when Hot doesn't have enough context                    ║
║                                                                   ║
╠═══════════════════════════════════════════════════════════════════╣
║                     TIER 3: COLD MEMORY                         ║
║                     (archive, <2s)                              ║
║                                                                   ║
║  Zvec collection: "cold"                                         ║
║  Full system history (unbounded)                                 ║
║  Compressed, heavily quantized                                   ║
║  Queried only for deep historical lookups                        ║
║                                                                   ║
╠═══════════════════════════════════════════════════════════════════╣
║                     GROUND TRUTH: FILES                         ║
║                                                                   ║
║  Markdown files remain source of truth (OpenClaw compatible)     ║
║  /var/lib/agentos/memory/MEMORY.md                               ║
║  /var/lib/agentos/memory/daily/2026-02-20.md                    ║
║  /var/lib/agentos/memory/systems/*.md                            ║
║  Zvec indexes are derived — always rebuildable from files        ║
║                                                                   ║
╚═══════════════════════════════════════════════════════════════════╝
```

### Why Three Tiers?

Your OS generates **thousands** of events per hour: process starts, file
changes, network connections, log lines, service states. You can't embed
all of them at full dimension in RAM forever.

But you also need the last 5 minutes of events **instantly** — when the
user says "why did that crash?" the agent needs to recall the error from
30 seconds ago without a 2-second disk seek.

**Hot** = always in RAM, always queried, auto-evicted after 24h.
**Warm** = disk-backed, queried when needed, evicted after 90 days.
**Cold** = archive, queried rarely, kept forever.

Events flow: **Hot → Warm → Cold** (time-based promotion).

---

## 3. WHAT GETS REMEMBERED (Event Taxonomy)

Every memory event has a structured schema:

```python
@dataclass
class MemoryEvent:
    id: str                    # UUID
    timestamp: datetime        # When it happened
    category: str              # See taxonomy below
    subcategory: str           # Finer classification
    actor: str                 # Who/what caused it: "user", "agent", "system", "app.X"
    summary: str               # Human-readable one-liner
    detail: str                # Full context (what gets embedded)
    metadata: dict             # Structured fields for scalar filtering
    # metadata examples:
    #   severity: "info" | "warning" | "error" | "critical"
    #   service: "postgresql" | "nginx" | ...
    #   package: "firefox" | "nodejs" | ...
    #   path: "/etc/nixos/configuration.nix"
    #   generation: 42
    #   exit_code: 1
    #   duration_ms: 1500
    #   tags: ["networking", "dns", "resolved"]
    embedding_semantic: list[float]  # 768-dim semantic embedding
    embedding_technical: list[float] # 768-dim technical/code embedding (optional)
```

### Event Taxonomy

| Category | Subcategories | What Gets Stored | Example |
|----------|--------------|------------------|---------|
| **conversation** | user_request, agent_response, clarification | Full prompt + response summary | User asked to install Postgres |
| **system.package** | install, remove, upgrade, rollback, search | Package name, version, reason, outcome | Installed postgresql-16, generation 43 |
| **system.config** | nixos_option, edit, rebuild, generation | Config diff, option path, old→new value | Changed services.postgresql.enable = true |
| **system.service** | start, stop, restart, fail, status_change | Service name, status, logs snippet | postgresql.service failed: port already in use |
| **system.process** | spawn, crash, oom_kill, high_cpu, high_mem | PID, command, resource usage, signals | Chrome PID 4521 OOM killed (4.2GB RSS) |
| **system.network** | connect, disconnect, dns_fail, firewall_change | Interface, IPs, ports, rules | WiFi connected to "HomeNet" via wlp2s0 |
| **system.storage** | mount, unmount, space_warning, permission | Path, size, usage, inode info | /home at 92% capacity, 8GB remaining |
| **system.hardware** | usb_attach, gpu_change, sensor_alert, battery | Device, vendor, driver, readings | USB: SanDisk Ultra attached at /dev/sdb1 |
| **diagnosis** | investigation, root_cause, resolution, workaround | Problem description, steps taken, fix | High CPU was caused by runaway node process |
| **file** | create, modify, delete, move, permission_change | Path, size, type, context of change | Created /home/user/project/main.py (Python) |
| **approval** | requested, granted, denied | Operation, requester, decision, reason | Approved: nixos-rebuild (adds Docker) |
| **security** | login, sudo, capability_grant, sandbox_violation | User, action, source, outcome | Sandboxed tool tried to access /etc/passwd |
| **error** | application, system, kernel, agent | Error message, stack trace, context | SQLite: database disk image is malformed |
| **cron** | scheduled, executed, failed, skipped | Timer name, command, result | Weekly nix-store --gc freed 12GB |
| **user_pattern** | work_session, preference, habit | Activity type, time, frequency | User typically runs Docker builds 9-11am |

### What Does NOT Get Stored

- Raw log lines (too noisy — only significant events extracted)
- File contents (only metadata + context, not the bytes)
- Passwords, tokens, keys (NEVER — detected and scrubbed)
- Continuous metric streams (sampled, not every data point)

---

## 4. THE EMBEDDING STRATEGY

### Two Embedding Models, Running Locally

**Why two?** A single embedding model can't serve all memory types well.
"The PostgreSQL service failed because port 5432 was already bound" needs
both semantic understanding (service failure) AND technical precision
(port 5432, PostgreSQL, binding error).

**Model 1: Semantic (general understanding)**
```
nomic-embed-text-v2-moe (GGUF Q8_0)
- 475M params, MoE (only 305M active)
- 512MB on disk, ~350MB in RAM
- 768 dimensions (truncatable to 256 via Matryoshka)
- 100+ languages
- 8192 token context
- Runs on CPU at ~50 embeddings/sec
- Runs on GPU at ~500 embeddings/sec
```

**Model 2: Technical (code, configs, error messages)**
```
nomic-embed-code (GGUF Q4_K_M)
- 7B params but quantized to 4-bit
- ~4.2GB on disk
- 768 dimensions
- Specialized for code, configs, error strings
- Runs on GPU; CPU fallback with reduced throughput
- OPTIONAL: only loaded if GPU available, otherwise
  semantic model handles both
```

**Fallback chain (same as OpenClaw, but enhanced):**
1. Local GPU (both models) — fastest, private
2. Local CPU (semantic only) — still fast, private
3. API fallback (Anthropic/OpenAI) — if user configures
4. BM25-only (no embeddings) — always works, keyword matching

### Embedding Pipeline

```
Event occurs → agentd creates MemoryEvent
                    ↓
            Summary + Detail text
                    ↓
         ┌──────────┴──────────┐
         ↓                     ↓
  Semantic Embed          Technical Embed
  (nomic-v2-moe)         (nomic-code, optional)
         ↓                     ↓
         └──────────┬──────────┘
                    ↓
            Zvec Insert (Hot tier)
            + Append to daily markdown log
```

**Batching**: Events are batched every 5 seconds (configurable).
Instead of embedding one-by-one, agentd accumulates events and
embeds them as a batch. At 50 embeddings/sec on CPU, a batch of
10 events takes 200ms — imperceptible.

**Deduplication**: Before embedding, hash the summary+detail text.
If the hash matches a recent event (within 5 minutes), skip embedding.
This prevents log spam from flooding the vector store.

---

## 5. ZVEC COLLECTIONS SCHEMA

### Hot Collection (RAM-resident)

```python
import zvec

hot_schema = zvec.CollectionSchema(
    name="hot",
    # Semantic vector: primary search dimension
    vectors=[
        zvec.VectorSchema(
            "semantic",
            zvec.DataType.VECTOR_FP32,
            768
        ),
        # Technical vector: optional, for code/config/error searches
        zvec.VectorSchema(
            "technical",
            zvec.DataType.VECTOR_FP32,
            768
        ),
    ],
    # Scalar fields for filtering DURING vector search
    scalars=[
        zvec.ScalarSchema("category", zvec.DataType.STRING),
        zvec.ScalarSchema("subcategory", zvec.DataType.STRING),
        zvec.ScalarSchema("actor", zvec.DataType.STRING),
        zvec.ScalarSchema("severity", zvec.DataType.STRING),
        zvec.ScalarSchema("timestamp", zvec.DataType.INT64),  # epoch ms
        zvec.ScalarSchema("service", zvec.DataType.STRING),
        zvec.ScalarSchema("tags", zvec.DataType.STRING),  # comma-separated
        zvec.ScalarSchema("summary", zvec.DataType.STRING),
        zvec.ScalarSchema("detail", zvec.DataType.STRING),
    ],
)

hot_collection = zvec.create_and_open(
    path="/var/lib/agentos/memory/zvec/hot",
    schema=hot_schema,
    # No quantization for hot tier — full precision, in RAM
)
```

### Warm Collection (Disk, Quantized)

```python
warm_schema = zvec.CollectionSchema(
    name="warm",
    vectors=[
        zvec.VectorSchema("semantic", zvec.DataType.VECTOR_FP32, 768),
        zvec.VectorSchema("technical", zvec.DataType.VECTOR_FP32, 768),
    ],
    scalars=[
        # Same as hot, plus:
        zvec.ScalarSchema("category", zvec.DataType.STRING),
        zvec.ScalarSchema("subcategory", zvec.DataType.STRING),
        zvec.ScalarSchema("actor", zvec.DataType.STRING),
        zvec.ScalarSchema("severity", zvec.DataType.STRING),
        zvec.ScalarSchema("timestamp", zvec.DataType.INT64),
        zvec.ScalarSchema("service", zvec.DataType.STRING),
        zvec.ScalarSchema("tags", zvec.DataType.STRING),
        zvec.ScalarSchema("summary", zvec.DataType.STRING),
        zvec.ScalarSchema("detail", zvec.DataType.STRING),
        zvec.ScalarSchema("hot_event_id", zvec.DataType.STRING),  # trace back
    ],
)

warm_collection = zvec.create_and_open(
    path="/var/lib/agentos/memory/zvec/warm",
    schema=warm_schema,
    # int8 quantization: 4x less memory, ~1% recall loss
    # Config: quantize_type="int8", m=50, ef_construction=200
)
```

### Cold Collection (Archive)

Same schema, but with aggressive quantization and no technical vectors
(only semantic). Rebuilt monthly from warm tier events.

---

## 6. THE RECALL PIPELINE (How Memories "Pop In")

This is the critical design: **how do memories automatically surface
when the user talks to the OS agent?**

### Step 1: Pre-Query (runs BEFORE the LLM sees the prompt)

When the user sends a message, BEFORE it reaches the LLM:

```
User: "Why is my system so slow today?"
                ↓
        agentd Memory Recall Pipeline
                ↓
    ┌───────────────────────────────────────────┐
    │ 1. Embed the user's query                 │
    │    semantic_vec = embed("slow system")     │
    │                                           │
    │ 2. Search Hot tier (last 24h)             │
    │    zvec.query(                            │
    │      VectorQuery("semantic", semantic_vec),│
    │      filter="timestamp > now-24h",        │
    │      topk=10                              │
    │    )                                      │
    │                                           │
    │ 3. If <3 good results, search Warm tier   │
    │    zvec.query(warm_collection, ...)       │
    │                                           │
    │ 4. If <3 good results, search Cold tier   │
    │    zvec.query(cold_collection, ...)       │
    │                                           │
    │ 5. Also: BM25 keyword search on summaries │
    │    (for exact matches the vectors miss)   │
    │                                           │
    │ 6. Merge + rerank (weighted fusion)       │
    │    0.6 × vector_score + 0.3 × bm25_score │
    │    + 0.1 × recency_boost                  │
    │                                           │
    │ 7. MMR diversity filter (λ=0.7)           │
    │    Remove near-duplicate memories         │
    │                                           │
    │ 8. Return top 6 memory chunks             │
    └───────────────────────────────────────────┘
                ↓
    Injected into system prompt as context:
    
    <system_memory>
    [2 minutes ago] Chrome PID 8821 consuming 3.8GB RAM,
    system swap usage at 94%. OOM killer triggered on
    electron process.
    
    [34 minutes ago] User ran `docker build` which spawned
    47 parallel processes, peak CPU at 98% across all cores.
    
    [Yesterday] Similar slowdown diagnosed as Docker
    container leak — 23 stopped containers consuming 12GB
    disk. Resolved with `docker system prune`.
    
    [Last week] Installed memory-heavy VS Code extensions
    (Pylance, GitLens, Remote SSH). Total VS Code memory
    footprint increased from 800MB to 2.1GB.
    </system_memory>
```

**The agent now has instant, relevant context BEFORE it even starts thinking.**

It doesn't need to run `htop` or `ps aux` to diagnose — it already KNOWS
what's happening because it's been watching and remembering.

### Step 2: Contextual Recall (during agent reasoning)

The agent can also explicitly search memory via the `memory_recall` tool:

```
Agent thinking: "The user mentioned Docker yesterday. Let me check..."
→ memory_recall({ query: "docker issues user reported", timeframe: "7d" })
→ Returns: Docker build OOM from Tuesday, container leak from last week,
  user's preference to use podman over docker for new projects
```

### Step 3: Post-Action Memory Write

After the agent acts, the result gets stored:

```
Agent diagnosed: "System slow because Docker build + Chrome + VS Code
eating 8GB combined. OOM killer triggered."

→ New MemoryEvent:
  category: "diagnosis"
  subcategory: "root_cause"
  summary: "System slowdown: Docker build + Chrome + VS Code memory pressure"
  detail: "Combined memory usage exceeded 8GB physical RAM. Docker build
           spawned 47 processes peaking at 98% CPU. Chrome tab at 3.8GB.
           VS Code with extensions at 2.1GB. OOM killer terminated electron.
           Resolution: killed Docker build, user switched to incremental builds."
  metadata: { severity: "warning", tags: ["memory", "docker", "chrome", "diagnosis"] }
```

Next time the user says "it's slow again," the agent IMMEDIATELY knows
to check Docker + Chrome + VS Code memory — no re-diagnosis needed.

---

## 7. CONTINUOUS SYSTEM WATCHERS

agentd runs background watchers that continuously generate MemoryEvents:

### 7.1 Process Watcher
```
Interval: every 30 seconds
What: Top 10 processes by CPU/RAM, any new high-resource processes
Stores: Only CHANGES (new high-CPU process, process died, memory spike)
Skip: Routine stable readings (no event if nothing changed)
```

### 7.2 Service Watcher
```
Interval: every 60 seconds
What: systemd unit state changes
Stores: Service started/stopped/failed/restarted
Skip: No event if all services unchanged
```

### 7.3 Journal Watcher
```
Method: `journalctl -f --output=json` (streaming, real-time)
What: Error/warning log lines from any service
Stores: Significant errors/warnings with service context
Skip: Info-level messages (unless related to tracked issue)
Filter: Dedup identical messages within 5-minute window
```

### 7.4 Network Watcher
```
Interval: every 60 seconds + event-driven (NetworkManager signals)
What: Interface changes, connectivity changes, DNS failures
Stores: WiFi connect/disconnect, IP changes, firewall modifications
Skip: Routine stable state
```

### 7.5 Filesystem Watcher
```
Method: inotify on configured paths (/etc/nixos, /home, key configs)
What: File creates/modifies/deletes on watched paths
Stores: File change with context (who changed it, from which command)
Skip: Temp files, build artifacts, .git internals
```

### 7.6 NixOS Generation Watcher
```
Trigger: After any nixos-rebuild
What: New generation number, config diff, added/removed packages
Stores: Full diff with human-readable summary
```

### 7.7 User Session Watcher
```
Trigger: Session start/end, application focus changes
What: What apps the user is working in, session patterns
Stores: "User switched from VS Code to Firefox at 14:32"
Skip: Rapid switches (< 3 second focus changes)
```

**Budget**: At steady state, these watchers produce ~50-200 events/hour.
At 200 events/hour × 768 dims × 4 bytes = ~600KB/hour of vector data.
That's ~14MB/day, ~420MB/month in the warm tier. Totally manageable.

---

## 8. THE BM25 SIDE (Keyword Precision)

Zvec handles vector search. But we also need exact keyword matching —
"error code 0x80070005" or "PID 12345" or "port 5432 in use."

**Solution: SQLite FTS5 alongside Zvec (like OpenClaw, but better scoped).**

```sql
-- /var/lib/agentos/memory/memory_fts.db
CREATE VIRTUAL TABLE memory_fts USING fts5(
    event_id,
    summary,
    detail,
    category,
    tags,
    tokenize='porter unicode61'
);

-- Inserted in parallel with every Zvec insert
-- Queried with BM25 ranking alongside vector search
```

### Hybrid Merge (OpenClaw-style but with recency)

```
final_score = (
    vector_weight × vector_score     # 0.55 default
  + bm25_weight × bm25_score        # 0.30 default
  + recency_weight × recency_score   # 0.15 default
)

where:
  recency_score = exp(-age_hours / half_life_hours)
  half_life_hours = 168 (7 days — recent events boosted)
```

This means:
- A semantically perfect match from a month ago scores ~0.55
- A keyword-exact match from yesterday scores ~0.30 + ~0.14 = 0.44
- A semantically good + keyword match from today scores ~0.50 + 0.25 + 0.15 = 0.90

**Recent, relevant memories always win. Old memories still findable.**

---

## 9. MEMORY-AWARE SYSTEM PROMPT INJECTION

The key to "memories popping in" is automatic injection into every prompt.
The user never asks "search your memory" — it just happens.

### Prompt Assembly Order

```
1. SOUL.md (agent identity — static)
2. AGENTS.md (capabilities — static)
3. TOOLS.md (available tools — static)

4. ── MEMORY INJECTION POINT ──
   
   <system_memory relevance="auto" count="6">
   [2m ago] Service postgresql.service started successfully (gen 45)
   [15m ago] User asked to set up Postgres; installed via nixos-rebuild
   [2h ago] System update: 12 packages upgraded, no issues
   [Yesterday] User preference: always use declarative NixOS config, not imperative
   [3 days ago] Docker + Chrome caused memory pressure; user prefers lean setups
   [Last month] User's project uses Python 3.12 + FastAPI + PostgreSQL stack
   </system_memory>

5. Current session conversation history
6. User's new message
```

### Adaptive Retrieval

Not every user message needs 6 memories. The pipeline adapts:

```
"Hi, how's it going?" → 0 memories (greeting, no context needed)
"What's using my CPU?" → 3-4 memories (recent process/service events)
"Set up a new Python project" → 5-6 memories (user preferences, past projects, installed tools)
"Continue what we were doing yesterday" → 6+ memories (session history, recent work)
```

**How it decides**: Classify the query type before retrieval:
- **Greeting/meta**: Skip memory retrieval entirely
- **Diagnostic**: Pull recent system events (time-weighted)
- **Task**: Pull user preferences + relevant past actions
- **Continuation**: Pull recent session memories + task context

---

## 10. OPENCLAW INTEGRATION: THE BRIDGE

OpenClaw has its own memory system. We don't replace it — we **wrap and extend** it.

### Option A: Custom Memory Backend (Recommended)

OpenClaw supports `memory.backend` configuration. We create a new backend:

```yaml
# OpenClaw config
memory:
  backend: agentos    # Our custom backend
  citations: auto
  agentos:
    socket: /run/agentos/agentd.sock
    autoRecall: true       # Inject memories into every prompt
    maxResults: 6
    tierPriority: [hot, warm, cold]
    hybridWeights:
      vector: 0.55
      bm25: 0.30
      recency: 0.15
```

The `agentos` memory backend:
1. Receives OpenClaw's `memory_search(query)` tool calls
2. Routes them to agentd's Zvec-powered recall pipeline
3. Returns results in OpenClaw's expected format (with citations)
4. Also handles `memory_get(path)` for direct file reads

**Plus**: agentd hooks into OpenClaw's session lifecycle:
- On session start → warm the hot tier, inject recent context
- On context compaction → flush important context to memory before trimming
- On session end → summarize and store session as a memory event

### Option B: OpenClaw Plugin (If backend API too complex)

If OpenClaw's backend API is too rigid, implement as a plugin that:
1. Intercepts every user message BEFORE it reaches the agent
2. Runs the recall pipeline
3. Prepends memory context to the message
4. Intercepts agent responses and extracts memory-worthy events

### The Memory Tools Exposed to OpenClaw

```typescript
// In agentos-bridge plugin:

gateway.registerTool('memory_recall', {
  description: 'Search system memory for relevant context. Returns memories sorted by relevance.',
  schema: {
    query: { type: 'string', description: 'What to search for' },
    timeframe: { type: 'string', description: 'How far back: "1h", "24h", "7d", "30d", "all"' },
    category: { type: 'string', description: 'Filter by category' },
    limit: { type: 'number', default: 6 }
  },
  async execute({ query, timeframe, category, limit }) {
    return await agentdClient.post('/memory/recall', { query, timeframe, category, limit });
  }
});

gateway.registerTool('memory_store', {
  description: 'Explicitly store something important in memory.',
  schema: {
    summary: { type: 'string' },
    detail: { type: 'string' },
    category: { type: 'string' },
    tags: { type: 'array', items: { type: 'string' } }
  },
  async execute({ summary, detail, category, tags }) {
    return await agentdClient.post('/memory/store', { summary, detail, category, tags });
  }
});

gateway.registerTool('memory_timeline', {
  description: 'Get a chronological view of recent system events.',
  schema: {
    hours: { type: 'number', default: 24 },
    category: { type: 'string', description: 'Filter by event type' }
  },
  async execute({ hours, category }) {
    return await agentdClient.post('/memory/timeline', { hours, category });
  }
});
```

---

## 11. agentd MEMORY API

### New Endpoints

```
POST /memory/ingest     # Ingest a new MemoryEvent (from watchers)
POST /memory/recall     # Search memory with hybrid retrieval
POST /memory/store      # Explicitly store a memory (from agent)
POST /memory/timeline   # Chronological event view
POST /memory/stats      # Collection stats (counts, sizes, health)
POST /memory/compact    # Force tier promotion (hot→warm→cold)
POST /memory/rebuild    # Rebuild indexes from ground-truth files
GET  /memory/health     # Embedding model status, tier sizes
```

### Internal Architecture (inside agentd)

```rust
// crates/agentd/src/memory/mod.rs

pub struct MemorySystem {
    // Zvec collections
    hot: ZvecCollection,       // In-process, RAM
    warm: ZvecCollection,      // In-process, disk
    cold: ZvecCollection,      // In-process, disk, compressed
    
    // BM25 for keyword search
    fts: SqliteFts,            // SQLite FTS5
    
    // Embedding models (loaded on startup)
    semantic_model: EmbeddingModel,    // nomic-embed-text-v2-moe
    technical_model: Option<EmbeddingModel>,  // nomic-embed-code (if GPU)
    
    // Background watchers
    watchers: Vec<SystemWatcher>,
    
    // Event queue (batched embedding)
    pending_events: Arc<Mutex<Vec<MemoryEvent>>>,
    
    // Configuration
    config: MemoryConfig,
}

impl MemorySystem {
    /// Called on every user prompt - returns relevant memories
    pub async fn auto_recall(&self, user_query: &str) -> Vec<MemoryChunk> {
        // 1. Classify query type
        let query_type = self.classify_query(user_query);
        if query_type == QueryType::Greeting { return vec![]; }
        
        // 2. Embed query
        let query_vec = self.semantic_model.embed(user_query).await;
        
        // 3. Search tiers with escalation
        let mut results = self.search_tier(&self.hot, &query_vec, 10).await;
        if results.len() < 3 {
            results.extend(self.search_tier(&self.warm, &query_vec, 10).await);
        }
        if results.len() < 3 {
            results.extend(self.search_tier(&self.cold, &query_vec, 5).await);
        }
        
        // 4. BM25 keyword search
        let bm25_results = self.fts.search(user_query, 10).await;
        
        // 5. Hybrid merge with recency
        let merged = self.hybrid_merge(results, bm25_results);
        
        // 6. MMR diversity filter
        let diverse = self.mmr_rerank(merged, 0.7);
        
        // 7. Return top N
        diverse.into_iter().take(self.config.max_recall_results).collect()
    }
    
    /// Background: batch embed and ingest pending events
    pub async fn flush_pending(&self) {
        let events = self.pending_events.lock().take();
        if events.is_empty() { return; }
        
        // Batch embed
        let texts: Vec<&str> = events.iter().map(|e| e.detail.as_str()).collect();
        let embeddings = self.semantic_model.embed_batch(&texts).await;
        
        // Insert into Zvec hot tier
        for (event, embedding) in events.iter().zip(embeddings.iter()) {
            self.hot.insert(event, embedding);
            self.fts.insert(event);
            self.append_to_daily_log(event);
        }
    }
    
    /// Nightly: promote hot→warm, warm→cold
    pub async fn tier_compaction(&self) {
        // Move events older than 24h from hot to warm
        // Move events older than 90d from warm to cold
        // Rebuild warm index with int8 quantization
    }
}
```

---

## 12. NESTING WITH THE LEDGER

The memory system and the audit ledger are SEPARATE but LINKED:

```
┌──────────────┐         ┌──────────────────┐
│   Ledger     │────────▶│  Memory System   │
│ (events.db)  │  event  │  (Zvec + FTS5)   │
│ hash-chained │  feed   │  searchable      │
│ append-only  │         │  embeddable      │
│ tamper-proof │         │  tiered          │
└──────────────┘         └──────────────────┘
```

**Ledger** = immutable audit trail. Every system mutation logged with
hash chain. Legal/compliance grade. Not searchable by semantics.

**Memory** = searchable knowledge. Derived from ledger events + system
watchers + conversation history. Optimized for instant recall.

When agentd logs an event to the ledger, it ALSO feeds it to the
memory ingestion pipeline. But the memory system can have additional
events that aren't in the ledger (like conversation summaries, user
preferences, learned patterns).

---

## 13. LEARNED PATTERNS (The "User Model")

Over time, the memory system builds a model of the user:

### Automatic Pattern Detection

```python
# Example patterns the system learns and stores as high-weight memories:

patterns = [
    MemoryEvent(
        category="user_pattern",
        subcategory="preference",
        summary="User prefers declarative NixOS config over imperative commands",
        detail="In 12 out of 14 package installations, user asked agent to edit "
               "configuration.nix rather than using nix-env. When agent suggested "
               "imperative install twice, user corrected to declarative.",
        metadata={"confidence": 0.92, "observations": 14}
    ),
    MemoryEvent(
        category="user_pattern",
        subcategory="schedule",
        summary="User's development sessions: Mon-Fri 9:00-12:00, 14:00-18:00",
        detail="Over 30 days, user consistently starts coding sessions in morning, "
               "takes lunch break, returns for afternoon session. Weekend usage is "
               "lighter, mostly browsing and media.",
        metadata={"confidence": 0.85, "observations": 30}
    ),
    MemoryEvent(
        category="user_pattern",
        subcategory="tech_stack",
        summary="User's primary stack: Python 3.12 + FastAPI + PostgreSQL + Docker",
        detail="User has created 3 projects with this stack in the last 2 months. "
               "Prefers poetry for Python packaging, pytest for testing, "
               "uvicorn for serving.",
        metadata={"confidence": 0.88, "observations": 8}
    ),
]
```

### How Patterns Are Detected

A nightly background job:
1. Pulls all conversation memories from the last 30 days
2. Clusters them by topic (using Zvec's vector similarity)
3. Identifies repeated themes (>3 occurrences = potential pattern)
4. Generates a pattern summary (using the LLM via agentd)
5. Stores with high weight so it always surfaces in recall

### Pattern Injection

User patterns get **priority injection** — they're always in the
system prompt, separate from regular memory recall:

```
<user_profile auto_updated="2026-02-20">
Preferences: Declarative NixOS config. Python/FastAPI/PostgreSQL stack.
Poetry for packaging. Pytest for testing. Prefers lean system (no bloat).
Schedule: Active dev Mon-Fri 9-12, 14-18. Light weekend usage.
Recent focus: Building API for project "Nexus", Docker containerization.
</user_profile>
```

---

## 14. NIXOS MODULE ADDITIONS

```nix
# In services.agentos.memory:
memory = {
  enable = mkOption { type = types.bool; default = true; };
  
  stateDir = mkOption {
    type = types.path;
    default = "/var/lib/agentos/memory";
  };
  
  embedding = {
    model = mkOption {
      type = types.str;
      default = "nomic-embed-text-v2-moe";
      description = "Local embedding model name (GGUF)";
    };
    quantization = mkOption {
      type = types.str;
      default = "Q8_0";
    };
    device = mkOption {
      type = types.enum [ "auto" "cpu" "cuda" "rocm" ];
      default = "auto";
    };
    batchSize = mkOption {
      type = types.int;
      default = 32;
    };
  };
  
  tiers = {
    hot = {
      maxAge = mkOption { type = types.str; default = "24h"; };
      maxVectors = mkOption { type = types.int; default = 50000; };
    };
    warm = {
      maxAge = mkOption { type = types.str; default = "90d"; };
      quantize = mkOption { type = types.str; default = "int8"; };
    };
    cold = {
      enabled = mkOption { type = types.bool; default = true; };
    };
  };
  
  recall = {
    autoInject = mkOption { type = types.bool; default = true; };
    maxResults = mkOption { type = types.int; default = 6; };
    weights = {
      vector = mkOption { type = types.float; default = 0.55; };
      bm25 = mkOption { type = types.float; default = 0.30; };
      recency = mkOption { type = types.float; default = 0.15; };
    };
    recencyHalfLifeHours = mkOption { type = types.int; default = 168; };
  };
  
  watchers = {
    process = { enable = mkOption { type = types.bool; default = true; }; interval = mkOption { type = types.int; default = 30; }; };
    service = { enable = mkOption { type = types.bool; default = true; }; interval = mkOption { type = types.int; default = 60; }; };
    journal = { enable = mkOption { type = types.bool; default = true; }; };
    network = { enable = mkOption { type = types.bool; default = true; }; };
    filesystem = { 
      enable = mkOption { type = types.bool; default = true; };
      watchPaths = mkOption { type = types.listOf types.str; default = [ "/etc/nixos" ]; };
    };
    nixos = { enable = mkOption { type = types.bool; default = true; }; };
  };
  
  patterns = {
    enable = mkOption { type = types.bool; default = true; };
    detectionInterval = mkOption { type = types.str; default = "daily"; };
    minObservations = mkOption { type = types.int; default = 3; };
  };
};
```

---

## 15. STORAGE BUDGET

### Per-Day Estimates

| Component | Size | Notes |
|-----------|------|-------|
| Hot vectors (50K × 768 × 4B) | ~150MB | RAM only, reused daily |
| Daily events (~200/hr × 24h) | ~4,800 events | Most days much less |
| Event embeddings (4,800 × 768 × 4B) | ~14MB | On disk |
| FTS5 entries | ~2MB | Text only |
| Daily markdown log | ~500KB | Human-readable |
| Embedding model (semantic) | ~512MB | Loaded once, shared |
| Embedding model (technical) | ~4.2GB | Optional, GPU only |

### Monthly

| Tier | Vectors | Disk | RAM |
|------|---------|------|-----|
| Hot | ~50K (rolling) | ~150MB | ~150MB |
| Warm | ~150K (90 days) | ~500MB | ~50MB (int8 quantized index) |
| Cold | ~1M+ (all time) | ~2GB | ~20MB (sparse index) |
| FTS5 | N/A | ~100MB | ~30MB |
| **Total** | | **~3GB** | **~250MB** |

This is **nothing** for a modern machine. The embedding model is the
biggest cost (512MB), and it's a one-time load.

---

## 16. WHAT "PERFECT MEMORY" LOOKS LIKE IN PRACTICE

### Scenario 1: Recurring Problem

```
Monday: "My system is slow"
Agent: Diagnoses Docker + Chrome memory pressure.
       Stores: diagnosis event with tags [memory, docker, chrome]

Wednesday: "It's slow again"
Agent: INSTANTLY recalls Monday's diagnosis.
       Says: "Last time this happened Monday, it was Docker + Chrome
       eating memory. Let me check... yes, Docker build running again
       (47 processes, 3.2GB). Chrome at 2.8GB across 23 tabs.
       Want me to stop the Docker build?"
```

No re-diagnosis. Instant pattern recognition.

### Scenario 2: Preference Learning

```
Week 1: User says "install postgres" → Agent suggests `nix-env -i`
        User: "No, add it to configuration.nix"
        
Week 2: User says "install redis" → Agent suggests `nix-env -i`
        User: "configuration.nix please"
        
Week 3: User says "install nginx"
Agent: "I'll add nginx to your configuration.nix and rebuild.
       I know you prefer declarative config."
       
       Memory: User pattern detected (confidence: 0.92)
```

### Scenario 3: Deep Historical Recall

```
User: "What was that weird network issue we had last month?"

Agent: Searches warm/cold memory → finds:
       "On January 23, your WiFi kept disconnecting every 4 minutes.
       Root cause was NetworkManager conflicting with wpa_supplicant
       after the NixOS 24.11 update. Fixed by setting
       networking.wireless.enable = false and using NetworkManager
       exclusively. Generation 38."
```

Perfectly recalled. Months later. With the exact config change.

### Scenario 4: Proactive Intelligence

```
Agent notices (via watchers):
- Disk usage at 89% (approaching 90% warning threshold)
- 15GB of old Docker images
- 8GB of old NixOS generations
- User's nightly backup cron has been failing silently for 3 days

On next interaction:
Agent: "Before we start — heads up on three things:
1. Your disk is at 89%. I can free ~23GB by pruning old Docker images
   and NixOS generations. Want me to?
2. Your nightly backup to /mnt/backup has been failing since Tuesday
   with 'permission denied'. The mount point permissions changed
   during last week's rebuild. I can fix the permissions.
3. I also noticed you haven't used postgresql-15 in 3 weeks but it's
   still running. Want me to stop it?"
```

The OS agent knows EVERYTHING. Proactively. Without being asked.

---

## 17. IMPLEMENTATION PRIORITY

| Step | What | Why First |
|------|------|-----------|
| 1 | Zvec Rust bindings (or Python subprocess) | Can't build anything without the engine |
| 2 | Hot tier collection + basic ingest | Start storing events immediately |
| 3 | Semantic embedding (nomic GGUF via llama.cpp) | Need vectors for search |
| 4 | Basic recall pipeline (vector search, no BM25) | Prove the concept works |
| 5 | FTS5 alongside Zvec for BM25 | Add keyword precision |
| 6 | Hybrid merge with recency scoring | Production-quality retrieval |
| 7 | System watchers (process, service, journal) | Continuous memory accumulation |
| 8 | OpenClaw integration (memory backend or plugin) | Memories "pop in" automatically |
| 9 | Warm tier + tier compaction | Scale beyond 24 hours |
| 10 | Pattern detection | User model / learning |
| 11 | Cold tier + archive | Full history |
| 12 | Proactive alerts | The agent volunteers information |

### Zvec Binding Strategy

Zvec has Python and Node.js bindings. agentd is Rust. Options:

**Option A: Python subprocess** (fastest to ship)
agentd spawns a long-running Python process that manages Zvec collections.
Communication via Unix socket or stdin/stdout JSON-RPC.
Pro: Works today. Con: Extra process.

**Option B: Zvec C++ FFI from Rust** (best performance)
Zvec is C++ under the hood. Use bindgen or cxx to call Proxima directly.
Pro: In-process, zero overhead. Con: Build complexity.

**Option C: Node.js Zvec in OpenClaw plugin** (simplest integration)
Zvec has an npm package. The agentos-bridge plugin manages Zvec directly.
agentd handles ledger + watchers, plugin handles vector search.
Pro: No Rust bindings needed. Con: Memory not in agentd's address space.

**Recommendation: Start with Option C (Node.js in plugin), migrate to
Option B (Rust FFI) when performance matters. Option A as fallback.**

---

## 18. CLAUDE CODE ADDITIONS

Add to CLAUDE.md:

```
## Memory System (Total Recall)

### Engine: Zvec (in-process vector DB)
- pip install zvec OR npm install @zvec/zvec
- Three-tier: Hot (RAM, 24h) → Warm (disk, 90d) → Cold (archive, forever)
- Zvec collections at /var/lib/agentos/memory/zvec/{hot,warm,cold}

### Embedding: Local GGUF models
- Primary: nomic-embed-text-v2-moe (Q8_0, 512MB, 768-dim)
- Optional: nomic-embed-code (Q4_K_M, 4.2GB, for code/configs)
- Via llama.cpp or node-llama-cpp

### Hybrid Search: Zvec vectors + SQLite FTS5 BM25
- Weighted fusion: 0.55 vector + 0.30 bm25 + 0.15 recency
- MMR diversity (λ=0.7) to avoid duplicate memories
- Scalar filters (category, severity, timestamp) pushed into Zvec index

### Auto-injection: Every user prompt gets relevant memories
- Pre-query recall pipeline runs BEFORE LLM sees the message
- Top 6 memories injected as <system_memory> block
- Zero user effort — memories just "pop in"

### Watchers: Background system event capture
- Process (30s), Service (60s), Journal (streaming), Network (60s)
- Filesystem (inotify), NixOS generations (on rebuild)
- Events batched, deduped, embedded, stored in hot tier

### Ground truth: Markdown files
- /var/lib/agentos/memory/daily/YYYY-MM-DD.md
- /var/lib/agentos/memory/MEMORY.md
- Zvec indexes are derived — always rebuildable from files
```
