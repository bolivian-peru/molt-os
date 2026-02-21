/**
 * osModa Memory Backend for OpenClaw.
 *
 * Replaces OpenClaw's built-in sqlite-vec memory with ZVEC-powered
 * OS-level memory. This backend is called automatically by OpenClaw
 * before every LLM turn to inject relevant memories into the prompt.
 *
 * Memory injection flow:
 * 1. User sends message → OpenClaw calls search() with the query
 * 2. We embed the query locally, search ZVEC + FTS5 via agentd
 * 3. Return relevant chunks → OpenClaw injects them as <system_memory>
 * 4. Claude sees memories as context text — never touches the vector DB
 */

import { AgentdClient } from "./agentd-client";

export interface MemoryChunk {
  id: string;
  summary: string;
  detail: string;
  category: string;
  timestamp: number;
  relevance_score: number;
  tags: string[];
}

export interface SearchOptions {
  maxResults?: number;
  timeframe?: string;
  category?: string;
}

export interface IndexMetadata {
  summary?: string;
  type?: string;
  actor?: string;
  tags?: string[];
}

export interface CompactionContext {
  messagesToFlush: unknown[];
  sessionId: string;
}

export class OsModaMemoryBackend {
  private client: AgentdClient;

  constructor(client: AgentdClient) {
    this.client = client;
  }

  /**
   * Called by OpenClaw before every LLM turn.
   * This is THE injection point where memories "pop in."
   *
   * The search runs entirely locally:
   * 1. agentd embeds the query with the local nomic model
   * 2. Searches ZVEC collection for semantic matches
   * 3. Searches SQLite FTS5 for BM25 keyword matches
   * 4. Merges with RRF (Reciprocal Rank Fusion)
   * 5. Applies relevance threshold and token budget cap (~1500 tokens)
   * 6. Returns top N chunks
   */
  async search(query: string, options: SearchOptions = {}): Promise<MemoryChunk[]> {
    try {
      const result = await this.client.post("/memory/recall", {
        query,
        max_results: options.maxResults || 10,
        timeframe: options.timeframe || "auto",
        category: options.category,
      });
      return (result as MemoryChunk[]) || [];
    } catch (err) {
      // Memory system failure should not break the agent
      // Fall back gracefully with no memories
      console.error("[osmoda-memory] Recall failed, continuing without memories:", err);
      return [];
    }
  }

  /**
   * Called by OpenClaw to get a specific memory file by path.
   * Used for direct file reads from the memory directory.
   */
  async get(path: string): Promise<string | null> {
    try {
      const result = await this.client.post("/memory/recall", {
        query: path,
        max_results: 1,
        timeframe: "all",
      });
      const chunks = result as MemoryChunk[];
      return chunks.length > 0 ? chunks[0].detail : null;
    } catch {
      return null;
    }
  }

  /**
   * Called when OpenClaw indexes new content (conversation turns,
   * file changes, etc.). We feed it into agentd's memory pipeline.
   */
  async index(content: string, metadata: IndexMetadata = {}): Promise<void> {
    try {
      await this.client.post("/memory/ingest", {
        event: {
          category: metadata.type || "conversation",
          subcategory: "indexed",
          actor: metadata.actor || "openclaw.gateway",
          summary: metadata.summary || content.slice(0, 200),
          detail: content,
          metadata: {
            tags: metadata.tags || [],
            timestamp: Date.now(),
          },
        },
      });
    } catch (err) {
      console.error("[osmoda-memory] Index failed:", err);
    }
  }

  /**
   * Called before context compaction — save important context
   * before OpenClaw trims the conversation history.
   *
   * This ensures that key information from the conversation
   * is preserved in long-term memory even when the context
   * window gets compressed.
   */
  async flush(context: CompactionContext): Promise<void> {
    try {
      // Store a summary of the messages being compacted
      const messageCount = context.messagesToFlush.length;
      const summary = `Session ${context.sessionId}: ${messageCount} messages compacted`;

      await this.client.post("/memory/store", {
        summary,
        detail: JSON.stringify(context.messagesToFlush),
        category: "session.compaction",
        tags: ["compaction", context.sessionId],
      });
    } catch (err) {
      console.error("[osmoda-memory] Flush failed:", err);
    }
  }
}
