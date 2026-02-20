# AgentOS Memory ↔ LLM Integration
## How Vector Search Reaches Claude When Claude Is a Remote API

---

## THE QUESTION

OpenClaw calls Claude via API (OAuth, Claude Max plan, or API key).
Claude runs on Anthropic's servers. Claude does NOT have a vector database.
Claude does NOT have access to our local Zvec collections.

**So how do memories "pop in"?**

---

## THE ANSWER IN ONE DIAGRAM

```
USER TYPES: "Why is my system slow?"
         │
         ▼
┌─────────────────────────────────────────────────────────────┐
│              OPENCLAW GATEWAY (local machine)                │
│                                                             │
│  1. Message arrives from user (WhatsApp/Terminal/WebChat)   │
│                                                             │
│  2. ════════════════════════════════════════════════════     │
│     ║  MEMORY RECALL (runs HERE, locally, BEFORE Claude) ║  │
│     ║                                                    ║  │
│     ║  a) Embed user query with local nomic model        ║  │
│     ║     "Why is my system slow?" → [0.23, -0.41, ...]  ║  │
│     ║                                                    ║  │
│     ║  b) Search Zvec Hot tier (in-process, <50ms)       ║  │
│     ║     → Chrome eating 3.8GB RAM (2 min ago)          ║  │
│     ║     → Docker build spawned 47 processes (34m ago)  ║  │
│     ║                                                    ║  │
│     ║  c) Search SQLite FTS5 for keywords (BM25)         ║  │
│     ║     → "slow" matches 3 past diagnosis events       ║  │
│     ║                                                    ║  │
│     ║  d) Merge + rerank → top 6 memories                ║  │
│     ════════════════════════════════════════════════════     │
│                                                             │
│  3. BUILD THE PROMPT (what Claude actually sees):           │
│     ┌─────────────────────────────────────────────┐        │
│     │ System prompt:                               │        │
│     │   SOUL.md (you are the OS)                   │        │
│     │   AGENTS.md (your capabilities)              │        │
│     │   TOOLS.md (available tools)                 │        │
│     │   Tool definitions (JSON schemas)            │        │
│     │                                              │        │
│     │   <system_memory>           ← INJECTED HERE  │        │
│     │   [2m ago] Chrome PID 8821: 3.8GB RAM        │        │
│     │   [34m ago] Docker build: 47 processes, 98%  │        │
│     │   [Yesterday] Similar slowdown was Docker     │        │
│     │   container leak. Fixed with docker prune.    │        │
│     │   [User preference] Prefers lean system.      │        │
│     │   </system_memory>                            │        │
│     │                                              │        │
│     │ User message:                                │        │
│     │   "Why is my system slow?"                   │        │
│     └─────────────────────────────────────────────┘        │
│                                                             │
│  4. SEND TO CLAUDE API ─────────────────────────────────┐  │
│                                                          │  │
└──────────────────────────────────────────────────────────┼──┘
                                                           │
                    INTERNET (HTTPS)                        │
                                                           ▼
┌──────────────────────────────────────────────────────────────┐
│                 ANTHROPIC SERVERS (Claude)                    │
│                                                              │
│  Claude receives ONE prompt with memories ALREADY INSIDE.    │
│  Claude has no idea these came from Zvec.                    │
│  Claude just sees text in its context window.                │
│                                                              │
│  Claude thinks: "I can see Chrome is eating 3.8GB and Docker │
│  had 47 processes. Last time this happened it was container  │
│  leak. Let me check both..."                                 │
│                                                              │
│  Claude responds with TOOL CALLS:                            │
│    → system_query({ query: "processes", sort: "ram" })       │
│    → system_query({ query: "docker.containers" })            │
│                                                              │
└──────────────┬───────────────────────────────────────────────┘
               │
               ▼ (tool call results streamed back)
┌──────────────────────────────────────────────────────────────┐
│              OPENCLAW GATEWAY (local machine)                 │
│                                                              │
│  5. Execute tool calls via agentd                            │
│     → agentd reads /proc, docker ps                          │
│     → Returns structured JSON                                │
│                                                              │
│  6. Send tool results back to Claude                         │
│                                                              │
│  7. Claude synthesizes and responds:                         │
│     "Your system is slow because Chrome is using 3.8GB       │
│      across 23 tabs and Docker has 47 build processes        │
│      running. Last time this happened we fixed it by..."     │
│                                                              │
│  8. ═══════════════════════════════════════════════           │
│     ║ MEMORY WRITE (after response, locally)     ║           │
│     ║                                            ║           │
│     ║ New event: diagnosis.root_cause            ║           │
│     ║ "System slow: Chrome 3.8GB + Docker 47     ║           │
│     ║  processes. Same pattern as yesterday."    ║           │
│     ║                                            ║           │
│     ║ → Embed → Insert into Zvec Hot tier        ║           │
│     ║ → Append to daily markdown log             ║           │
│     ═══════════════════════════════════════════════           │
│                                                              │
│  9. Deliver response to user                                 │
│                                                              │
└──────────────────────────────────────────────────────────────┘
```

**The critical insight: Claude never touches the vector database.
The vector search runs 100% locally, BEFORE and AFTER the API call.
Claude just sees text.**

---

## EXACTLY WHERE IN OPENCLAW THIS HAPPENS

OpenClaw already has this architecture. We're extending it, not inventing it.

### The Existing Flow (OpenClaw today)

```
src/agents/pi-embedded-runner/run/attempt.ts
  → buildAgentSystemPrompt()          // Assembles everything Claude will see
    → src/agents/system-prompt.ts
      → Injects: SOUL.md, AGENTS.md, TOOLS.md, Skills
      → Injects: Tool definitions
      → Injects: Memory context (from memory search)     ← THIS IS WHERE WE HOOK
      → Injects: Bootstrap files
    → Sends to model provider (Claude API via OAuth)
```

OpenClaw's `buildAgentSystemPrompt()` already queries `memory_search` to
find relevant past context and injects it. The function at line ~164 of
`system-prompt.ts` builds sections including "Memory and Reactions when
enabled."

### What We Change

We replace OpenClaw's default SQLite memory backend with our Zvec-powered
system. OpenClaw supports `memory.backend` config — we register ours:

```yaml
# OpenClaw config.yaml
memory:
  backend: agentos          # Our custom backend instead of "builtin" or "qmd"
  citations: auto
```

And we implement the backend interface that OpenClaw expects:

```typescript
// packages/agentos-bridge/memory-backend.ts

import { MemoryBackend } from '@openclaw/types';

export class AgentOSMemoryBackend implements MemoryBackend {
  
  private agentdSocket: string;  // Unix socket to agentd
  
  /**
   * Called by OpenClaw before every LLM turn.
   * This is THE injection point where memories "pop in."
   */
  async search(query: string, options: SearchOptions): Promise<MemoryChunk[]> {
    // agentd does the heavy lifting:
    // 1. Embeds query with local nomic model
    // 2. Searches Zvec (Hot → Warm → Cold escalation)
    // 3. Searches FTS5 for BM25 keywords
    // 4. Merges with recency weighting
    // 5. MMR diversity filter
    // 6. Returns top N chunks
    
    const response = await fetch(`http://unix:${this.agentdSocket}:/memory/recall`, {
      method: 'POST',
      body: JSON.stringify({
        query,
        maxResults: options.maxResults || 6,
        timeframe: 'auto',     // Hot first, escalate if needed
      }),
    });
    
    return response.json();  // Returns MemoryChunk[] in OpenClaw's format
  }
  
  /**
   * Called by OpenClaw to get a specific memory file.
   * Delegates to agentd for file reads.
   */
  async get(path: string): Promise<string | null> {
    const response = await fetch(`http://unix:${this.agentdSocket}:/memory/file`, {
      method: 'POST',
      body: JSON.stringify({ path }),
    });
    return response.text();
  }
  
  /**
   * Called when OpenClaw indexes new content.
   * We feed it into agentd's Zvec pipeline.
   */
  async index(content: string, metadata: IndexMetadata): Promise<void> {
    await fetch(`http://unix:${this.agentdSocket}:/memory/ingest`, {
      method: 'POST',
      body: JSON.stringify({
        summary: metadata.summary || content.slice(0, 200),
        detail: content,
        category: 'conversation',
        subcategory: metadata.type || 'general',
        actor: metadata.actor || 'user',
        tags: metadata.tags || [],
      }),
    });
  }
  
  /**
   * Called before context compaction — save important context
   * before OpenClaw trims the conversation history.
   */
  async flush(context: CompactionContext): Promise<void> {
    await fetch(`http://unix:${this.agentdSocket}:/memory/flush`, {
      method: 'POST',
      body: JSON.stringify({
        messages: context.messagesToFlush,
        sessionId: context.sessionId,
      }),
    });
  }
}
```

### Registration as OpenClaw Plugin

```typescript
// packages/agentos-bridge/index.ts

import { AgentOSMemoryBackend } from './memory-backend';

export default function agentOSBridge(gateway) {
  const agentdSocket = process.env.AGENTOS_SOCKET || '/run/agentos/agentd.sock';
  
  // 1. Register memory backend (replaces sqlite-vec)
  gateway.registerMemoryBackend('agentos', new AgentOSMemoryBackend(agentdSocket));
  
  // 2. Register tools (system_query, system_mutate, etc.)
  gateway.registerTool('system_query', { /* ... */ });
  gateway.registerTool('system_mutate', { /* ... */ });
  gateway.registerTool('nix_rebuild', { /* ... */ });
  gateway.registerTool('sandbox_exec', { /* ... */ });
  
  // 3. Register EXPLICIT memory tools (agent can search proactively)
  gateway.registerTool('memory_recall', {
    description: 'Search OS memory for relevant context. Finds past events, diagnoses, configs, errors.',
    schema: {
      query: { type: 'string', description: 'What to search for' },
      timeframe: { type: 'string', enum: ['1h', '24h', '7d', '30d', '90d', 'all'], default: '7d' },
      category: { type: 'string', description: 'Filter: conversation, diagnosis, system.*, error, file' },
    },
    async execute({ query, timeframe, category }) {
      return await agentdClient.post('/memory/recall', { query, timeframe, category });
    }
  });
  
  gateway.registerTool('memory_store', {
    description: 'Explicitly store something important in long-term memory.',
    schema: {
      summary: { type: 'string' },
      detail: { type: 'string' },
      category: { type: 'string' },
      tags: { type: 'array', items: { type: 'string' } },
    },
    async execute({ summary, detail, category, tags }) {
      return await agentdClient.post('/memory/store', { summary, detail, category, tags });
    }
  });
  
  // 4. Hook into session lifecycle
  gateway.on('session:start', async (session) => {
    // Warm the hot tier — pre-load recent context
    await agentdClient.post('/memory/warm', { sessionId: session.id });
  });
  
  gateway.on('session:before_compact', async (context) => {
    // Before OpenClaw trims conversation history,
    // flush important context to long-term memory
    await agentdClient.post('/memory/flush', {
      messages: context.messagesToCompact,
      sessionId: context.sessionId,
    });
  });
}
```

---

## THE TWO PATHS MEMORY REACHES CLAUDE

### Path 1: AUTOMATIC (every single prompt, zero user effort)

```
User says anything
       │
       ▼
OpenClaw's buildAgentSystemPrompt()
       │
       ├─ Calls our AgentOSMemoryBackend.search(userQuery)
       │     │
       │     ▼
       │  agentd /memory/recall
       │     │
       │     ├─ Local nomic model embeds the query (50ms)
       │     ├─ Zvec Hot tier search (20ms)
       │     ├─ FTS5 BM25 search (10ms)
       │     ├─ Hybrid merge + rerank (5ms)
       │     └─ Returns 6 memory chunks
       │
       ├─ Formats as text block:
       │     <system_memory>
       │     [2m ago] Chrome eating 3.8GB...
       │     [34m ago] Docker build running...
       │     </system_memory>
       │
       ├─ Inserts into system prompt
       │
       └─ Sends complete prompt to Claude API
              │
              ▼
         Claude sees memories as CONTEXT TEXT
         Claude responds with full awareness
```

**This happens on EVERY turn. The user never asks for it.**
**Total latency added: ~85ms (imperceptible).**

### Path 2: EXPLICIT (agent proactively searches)

```
Claude is reasoning about something and needs more context.
Claude decides to call the memory_recall tool.
       │
       ▼
Claude's response includes:
  tool_call: memory_recall({
    query: "docker container issues this month",
    timeframe: "30d",
    category: "diagnosis"
  })
       │
       ▼
OpenClaw executes the tool call
       │
       ▼
agentd /memory/recall with deeper search
       │
       ├─ Searches Warm + Cold tiers (not just Hot)
       ├─ Broader time window
       ├─ Category-filtered
       └─ Returns detailed results
       │
       ▼
Tool results sent back to Claude in next turn
       │
       ▼
Claude incorporates into its reasoning:
  "I found 3 Docker-related diagnoses this month..."
```

**Both paths work together.** Path 1 gives Claude immediate context.
Path 2 lets Claude dig deeper when needed.

---

## THE OAUTH / CLAUDE MAX FLOW SPECIFICALLY

When using Claude Max plan via OAuth (which is how most OpenClaw users
connect to Claude):

```
┌─────────────────────────────────────────────────────────┐
│ USER'S MACHINE                                          │
│                                                         │
│ OpenClaw Gateway (:18789)                               │
│   │                                                     │
│   ├─ agentd (memory/tools/ledger)                       │
│   │   ├─ Zvec (in-process vector DB)                    │
│   │   ├─ nomic embedding model (GGUF, local)            │
│   │   ├─ SQLite FTS5 (keyword search)                   │
│   │   └─ System watchers (continuous events)            │
│   │                                                     │
│   ├─ Pi Agent Runtime (embedded in Gateway)             │
│   │   ├─ buildAgentSystemPrompt() ← memory injected    │
│   │   ├─ Tool execution engine                          │
│   │   └─ Session management                             │
│   │                                                     │
│   └─ Model Provider: Claude via OAuth                   │
│       │                                                 │
│       │  POST https://api.anthropic.com/v1/messages     │
│       │  Authorization: Bearer <oauth_token>            │
│       │  Body: {                                        │
│       │    model: "claude-opus-4-6",                    │
│       │    system: "<FULL PROMPT WITH MEMORIES>",       │
│       │    messages: [...conversation...],              │
│       │    tools: [...tool_definitions...]              │
│       │  }                                              │
│       │                                                 │
└───────┼─────────────────────────────────────────────────┘
        │
        ▼
┌─────────────────────────────────────────────────────────┐
│ ANTHROPIC API                                           │
│                                                         │
│ Claude receives the prompt. The system message contains │
│ the memories as plain text. Claude has no idea they     │
│ came from Zvec. It just sees helpful context.           │
│                                                         │
│ Claude can also call tools:                             │
│   - memory_recall (explicit deeper search)              │
│   - system_query (live system data)                     │
│   - system_mutate (change things)                       │
│   - nix_rebuild (rebuild NixOS)                         │
│   - sandbox_exec (run untrusted code)                   │
│                                                         │
│ Tool calls are sent back to OpenClaw for execution.     │
│ Tool results are sent back to Claude for next turn.     │
│                                                         │
└─────────────────────────────────────────────────────────┘
```

**Key point: The OAuth token authenticates OpenClaw's Gateway to
Anthropic's API. The entire memory/tool system is between the user
and the API call. Claude is stateless — it receives a new, complete
prompt with memories on every single turn.**

---

## WHAT CLAUDE ACTUALLY SEES (Prompt Example)

Here is a realistic prompt that Claude receives via the API.
Notice: memories are just TEXT. Claude doesn't know about Zvec.

```
[SYSTEM PROMPT]

You are AgentOS. You ARE the operating system.

# Identity
If SOUL.md is present, embody its persona...

# Project Context

## SOUL.md
You are not an assistant running on a computer. You ARE the computer.
Calm. Competent. Omniscient about the system. Never flustered.
Diagnose before fixing. Explain before changing. Rollback on failure.

## AGENTS.md  
Full system access via agentd. Every process, file, network connection,
service, config file — you see it all, you control it all.

## TOOLS.md
[agentd endpoints documented...]

# Available Tools
- system_query: Query system state (processes, services, network, disk, logs)
- system_mutate: Modify system state (requires approval for destructive ops)
- nix_rebuild: Rebuild NixOS configuration
- sandbox_exec: Execute tool in bubblewrap sandbox
- memory_recall: Search OS memory for past events and context
- memory_store: Save important information to long-term memory
- read: Read file contents
- write: Write to file
- exec: Execute shell command
- browser: Open browser with CDP

# System Memory (auto-retrieved)

<system_memory retrieved_at="2026-02-20T14:32:15Z" source="zvec_hot">
[2 minutes ago | system.process | severity:warning]
Chrome PID 8821 consuming 3.8GB RAM across 23 tabs.
System swap usage at 94%. OOM killer triggered on electron process.

[34 minutes ago | system.process | severity:info]
User ran `docker build -t myapp .` which spawned 47 parallel
compilation processes. Peak CPU at 98% across all 8 cores.
Build still in progress.

[Yesterday | diagnosis.resolution | severity:info]
Similar slowdown diagnosed as Docker container leak — 23 stopped
containers consuming 12GB disk. Resolved with `docker system prune`.
User approved cleanup.

[3 days ago | user_pattern.preference | confidence:0.92]
User prefers lean system setups. Has declined installing heavy
IDEs twice. Prefers terminal-based workflows.

[Last week | system.config.generation | generation:44]
Installed memory-heavy VS Code extensions (Pylance, GitLens).
Total VS Code memory footprint increased from 800MB to 2.1GB.
User was informed but kept extensions.

[User profile]
Stack: Python 3.12, FastAPI, PostgreSQL, Docker.
Preference: Declarative NixOS config. Lean system.
Active hours: Mon-Fri 9-12, 14-18.
</system_memory>

# Current Date & Time
Timezone: Asia/Bangkok

[USER MESSAGE]
Why is my system slow today?
```

**Claude reads this and KNOWS:**
- Chrome is eating 3.8GB right now
- Docker build is running 47 processes at 98% CPU
- Yesterday had a similar issue (Docker container leak)
- The user likes lean systems
- VS Code extensions added 1.3GB recently

Claude doesn't need to run `htop` first. It already has the context.
It can jump straight to: "Your system is slow because of three things..."

---

## THE COMPLETE EVENT LIFECYCLE

```
TIME    EVENT                           WHERE IT LIVES
────    ─────                           ──────────────
t=0     Chrome opens 15 new tabs        → agentd process watcher detects
        Chrome RAM: 800MB → 3.8GB         memory spike
                                        → MemoryEvent created
                                        → Embedded locally (nomic, 50ms)
                                        → Inserted into Zvec Hot tier
                                        → Appended to daily markdown
                                        → Logged in hash-chained ledger

t=5m    Docker build starts             → agentd process watcher detects
        47 processes, 98% CPU             high CPU
                                        → MemoryEvent created
                                        → Same pipeline as above

t=10m   System swap at 94%              → agentd resource watcher
        OOM killer fires                → MemoryEvent (severity: warning)

t=12m   User: "Why is my system slow?"  → OpenClaw receives message
                                        → BEFORE calling Claude:
                                          - Embed query (50ms)
                                          - Search Zvec Hot (20ms)
                                          - Search FTS5 (10ms)
                                          - Merge + rerank (5ms)
                                          - Get 6 relevant memories
                                        → Build prompt with memories
                                        → POST to Claude API
                                        → Claude sees Chrome + Docker +
                                          OOM + yesterday's diagnosis
                                        → Claude responds intelligently
                                        → AFTER response:
                                          - New MemoryEvent (diagnosis)
                                          - Embedded + stored in Zvec
                                          - Logged in ledger

t=next  User: "It's slow again"         → Same pipeline
                                        → Zvec Hot instantly returns the
                                          diagnosis from t=12m
                                        → Claude: "Same issue as 10
                                          minutes ago — Chrome + Docker"
                                        → ZERO re-diagnosis needed
```

---

## WHY THIS IS BETTER THAN WHAT EXISTS

### vs. OpenClaw's Built-in Memory (sqlite-vec)

| Aspect | OpenClaw Default | AgentOS Total Recall |
|--------|-----------------|---------------------|
| What it remembers | Conversations | EVERYTHING (processes, services, configs, errors, files, user patterns) |
| Scale | ~50K chunks | 10M+ vectors across 3 tiers |
| Search speed | Adequate | Blazing (Zvec: 8500+ QPS) |
| Automatic recall | On demand via tool | Injected EVERY turn automatically |
| System awareness | None (chat-only) | Full system telemetry via watchers |
| Pattern learning | None | Automatic user model over time |
| Historical depth | Days/weeks | Forever (cold tier) |
| Proactive alerts | Never | Volunteers information on issues |

### vs. Mem0 / MemGPT / Other Agent Memory Systems

Those systems are designed for **chatbots**. We're building memory for
an **operating system**. The difference:

- Chatbot memory: "The user likes Italian food"
- OS memory: "PostgreSQL failed at 3am because the disk hit 99% after
  a Docker build left 15GB of intermediate layers. The disk was on
  /dev/sda2, an ext4 partition. The fix was docker system prune followed
  by adding a weekly cron job for cleanup. Generation 42."

---

## IMPLEMENTATION IN AGENTD (Rust)

The Zvec integration happens in agentd via Python subprocess or FFI:

### Option A: Python Sidecar (Ship First)

```rust
// crates/agentd/src/memory/zvec_sidecar.rs

use tokio::process::Command;
use serde::{Deserialize, Serialize};

/// Manages a long-running Python process that owns the Zvec collections
pub struct ZvecSidecar {
    process: tokio::process::Child,
    stdin: tokio::io::BufWriter<tokio::process::ChildStdin>,
    stdout: tokio::io::BufReader<tokio::process::ChildStdout>,
}

impl ZvecSidecar {
    pub async fn start(config: &MemoryConfig) -> Result<Self> {
        let mut child = Command::new("python3")
            .arg("-m")
            .arg("agentos_memory")  // Our Python package
            .arg("--state-dir")
            .arg(&config.state_dir)
            .arg("--embedding-model")
            .arg(&config.embedding_model)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()?;
        
        // JSON-RPC over stdin/stdout
        Ok(Self { /* ... */ })
    }
    
    pub async fn recall(&mut self, query: &str, opts: RecallOptions) -> Result<Vec<MemoryChunk>> {
        self.send_rpc("recall", json!({ "query": query, "opts": opts })).await
    }
    
    pub async fn ingest(&mut self, event: &MemoryEvent) -> Result<()> {
        self.send_rpc("ingest", json!({ "event": event })).await
    }
}
```

### The Python Sidecar Process

```python
# packages/agentos-memory/agentos_memory/__main__.py

import sys
import json
import zvec
from sentence_transformers import SentenceTransformer  # or llama-cpp-python

class MemoryEngine:
    def __init__(self, state_dir, model_name):
        self.model = SentenceTransformer(model_name)  # Or GGUF via llama.cpp
        self.hot = zvec.open(f"{state_dir}/zvec/hot")
        self.warm = zvec.open(f"{state_dir}/zvec/warm")
        # FTS5 connection
        self.fts = sqlite3.connect(f"{state_dir}/memory_fts.db")
    
    def recall(self, query, opts):
        # 1. Embed query
        query_vec = self.model.encode(query).tolist()
        
        # 2. Search Zvec Hot
        hot_results = self.hot.query(
            zvec.VectorQuery("semantic", vector=query_vec),
            topk=opts.get('limit', 10),
            # Scalar filter: only events from the right timeframe
            filter=f"timestamp > {self.time_threshold(opts.get('timeframe', '24h'))}"
        )
        
        # 3. Escalate if needed
        if len(hot_results) < 3:
            warm_results = self.warm.query(
                zvec.VectorQuery("semantic", vector=query_vec),
                topk=10,
            )
            hot_results.extend(warm_results)
        
        # 4. BM25 keyword search
        bm25_results = self.fts_search(query)
        
        # 5. Hybrid merge
        merged = self.hybrid_merge(hot_results, bm25_results, opts)
        
        # 6. MMR diversity
        diverse = self.mmr_rerank(merged, lambda_param=0.7)
        
        return diverse[:opts.get('limit', 6)]
    
    def ingest(self, event):
        # 1. Embed
        vec = self.model.encode(event['detail']).tolist()
        
        # 2. Insert into Zvec Hot
        self.hot.insert([zvec.Doc(
            id=event['id'],
            vectors={"semantic": vec},
            scalars={
                "category": event['category'],
                "timestamp": event['timestamp'],
                "severity": event.get('severity', 'info'),
                "summary": event['summary'],
                "detail": event['detail'],
                "tags": ",".join(event.get('tags', [])),
            }
        )])
        
        # 3. Insert into FTS5
        self.fts.execute(
            "INSERT INTO memory_fts VALUES (?, ?, ?, ?, ?)",
            (event['id'], event['summary'], event['detail'],
             event['category'], ",".join(event.get('tags', [])))
        )

# JSON-RPC loop over stdin/stdout
engine = MemoryEngine(sys.argv[2], sys.argv[4])
for line in sys.stdin:
    request = json.loads(line)
    method = request['method']
    params = request['params']
    
    if method == 'recall':
        result = engine.recall(params['query'], params.get('opts', {}))
    elif method == 'ingest':
        result = engine.ingest(params['event'])
    
    print(json.dumps({"id": request['id'], "result": result}), flush=True)
```

### Option B: Node.js Zvec in OpenClaw Plugin (Alternative)

Since Zvec has `npm install @zvec/zvec`, we can run it directly inside
the OpenClaw plugin:

```typescript
// packages/agentos-bridge/memory-engine.ts

import * as zvec from '@zvec/zvec';

export class MemoryEngine {
  private hot: zvec.Collection;
  private warm: zvec.Collection;
  
  async init(stateDir: string) {
    const hotSchema = new zvec.CollectionSchema({
      name: "hot",
      vectors: [
        new zvec.VectorSchema("semantic", zvec.DataType.VECTOR_FP32, 768),
      ],
      // ... scalar schemas
    });
    
    this.hot = await zvec.createAndOpen({
      path: `${stateDir}/zvec/hot`,
      schema: hotSchema,
    });
  }
  
  async recall(queryVec: number[], opts: RecallOptions): Promise<MemoryChunk[]> {
    const results = await this.hot.query(
      new zvec.VectorQuery("semantic", queryVec),
      { topk: 10, filter: `timestamp > ${opts.since}` }
    );
    return results;
  }
}
```

**For embedding in Node.js:** Use `node-llama-cpp` (same as OpenClaw's
local embedding) or call agentd's embedding endpoint.

---

## SUMMARY: THE COMPLETE PICTURE

```
┌─────────────────────────────────────────────────────────────┐
│ THE LLM (Claude) NEVER TOUCHES THE VECTOR DB.              │
│                                                             │
│ Vector search is a LOCAL operation that happens             │
│ INSIDE OpenClaw/agentd BEFORE the API call.                 │
│                                                             │
│ Results are injected as TEXT into Claude's prompt.           │
│ Claude sees memories as context. That's it.                 │
│                                                             │
│ Claude can also CALL memory_recall as a tool                │
│ for deeper searches. Same principle: local search,          │
│ results returned as text to Claude's context.               │
│                                                             │
│ This works identically whether Claude runs via:             │
│   - OAuth (Claude Max plan)                                 │
│   - API key (direct Anthropic API)                          │
│   - Any other provider (OpenAI, Gemini, local LLM)         │
│                                                             │
│ Because the memory system is provider-agnostic.             │
│ It just injects text into whatever prompt goes out.         │
└─────────────────────────────────────────────────────────────┘
```
