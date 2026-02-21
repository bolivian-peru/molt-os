/**
 * ZVEC Memory Engine — manages the vector collection for osModa memory.
 *
 * M0 strategy: Single ZVEC collection via @zvec/zvec npm package.
 * Runs in-process within the OpenClaw plugin (no separate daemon).
 * agentd handles the ledger and event routing; this handles vector search.
 *
 * Architecture:
 * - Single collection (tiering deferred to M2)
 * - nomic-embed-text-v2-moe for embeddings (768-dim)
 * - SQLite FTS5 for BM25 keyword search (managed by agentd)
 * - RRF (Reciprocal Rank Fusion) for hybrid merge
 * - LRU cache for query embeddings (5-minute TTL)
 * - Token budget cap: ~1500 tokens per injection
 */

// Note: @zvec/zvec types will be available when the package is installed
// For now we define the interface we expect

interface ZvecDocument {
  id: string;
  vectors: Record<string, number[]>;
  scalars: Record<string, string | number>;
}

interface ZvecQueryResult {
  id: string;
  score: number;
  scalars: Record<string, string | number>;
}

interface ZvecCollection {
  insert(docs: ZvecDocument[]): Promise<void>;
  query(
    vectorName: string,
    vector: number[],
    options: { topk: number; filter?: string },
  ): Promise<ZvecQueryResult[]>;
  count(): Promise<number>;
  close(): Promise<void>;
}

interface CacheEntry {
  vector: number[];
  timestamp: number;
}

const CACHE_TTL_MS = 5 * 60 * 1000; // 5 minutes
const MAX_CACHE_SIZE = 100;
const TOKEN_BUDGET = 1500;
const APPROX_CHARS_PER_TOKEN = 4;

export class MemoryEngine {
  private collection: ZvecCollection | null = null;
  private stateDir: string;
  private queryCache: Map<string, CacheEntry> = new Map();

  constructor(stateDir: string) {
    this.stateDir = stateDir;
  }

  /**
   * Initialize the ZVEC collection.
   * Creates or opens the collection at the configured state directory.
   */
  async init(): Promise<void> {
    try {
      // Dynamic import — @zvec/zvec may not be installed in dev
      const zvec = await import("@zvec/zvec");

      this.collection = await zvec.createAndOpen({
        path: `${this.stateDir}/zvec/main`,
        schema: {
          name: "main",
          vectors: [
            { name: "semantic", dataType: "vector_fp32", dimension: 768 },
          ],
          scalars: [
            { name: "category", dataType: "string" },
            { name: "subcategory", dataType: "string" },
            { name: "actor", dataType: "string" },
            { name: "severity", dataType: "string" },
            { name: "timestamp", dataType: "int64" },
            { name: "summary", dataType: "string" },
            { name: "detail", dataType: "string" },
            { name: "tags", dataType: "string" },
          ],
        },
      }) as ZvecCollection;

      console.log("[memory-engine] ZVEC collection initialized");
    } catch (err) {
      console.warn(
        "[memory-engine] ZVEC not available, memory search will use ledger fallback:",
        err,
      );
      this.collection = null;
    }
  }

  /**
   * Insert a document into the vector collection.
   */
  async insert(
    id: string,
    embedding: number[],
    scalars: Record<string, string | number>,
  ): Promise<void> {
    if (!this.collection) return;

    await this.collection.insert([
      {
        id,
        vectors: { semantic: embedding },
        scalars,
      },
    ]);
  }

  /**
   * Search the collection with a query vector.
   * Applies token budget cap to limit total injection size.
   */
  async search(
    queryVector: number[],
    topk: number = 10,
    filter?: string,
  ): Promise<ZvecQueryResult[]> {
    if (!this.collection) return [];

    const results = await this.collection.query("semantic", queryVector, {
      topk,
      filter,
    });

    // Apply token budget cap
    return this.applyTokenBudget(results);
  }

  /**
   * Get cached query embedding or return null.
   */
  getCachedEmbedding(query: string): number[] | null {
    const entry = this.queryCache.get(query);
    if (!entry) return null;

    if (Date.now() - entry.timestamp > CACHE_TTL_MS) {
      this.queryCache.delete(query);
      return null;
    }

    return entry.vector;
  }

  /**
   * Cache a query embedding.
   */
  cacheEmbedding(query: string, vector: number[]): void {
    // Evict oldest entries if cache is full
    if (this.queryCache.size >= MAX_CACHE_SIZE) {
      const oldest = [...this.queryCache.entries()].sort(
        (a, b) => a[1].timestamp - b[1].timestamp,
      )[0];
      if (oldest) {
        this.queryCache.delete(oldest[0]);
      }
    }

    this.queryCache.set(query, { vector, timestamp: Date.now() });
  }

  /**
   * Apply token budget cap to search results.
   * Ensures we don't inject more than ~1500 tokens of memory context.
   */
  private applyTokenBudget(results: ZvecQueryResult[]): ZvecQueryResult[] {
    const maxChars = TOKEN_BUDGET * APPROX_CHARS_PER_TOKEN;
    let totalChars = 0;
    const budgeted: ZvecQueryResult[] = [];

    for (const result of results) {
      const summary = String(result.scalars.summary || "");
      const detail = String(result.scalars.detail || "");
      const entryChars = summary.length + detail.length;

      if (totalChars + entryChars > maxChars && budgeted.length > 0) {
        break;
      }

      totalChars += entryChars;
      budgeted.push(result);
    }

    return budgeted;
  }

  /**
   * Get collection statistics.
   */
  async stats(): Promise<{ count: number; initialized: boolean }> {
    if (!this.collection) {
      return { count: 0, initialized: false };
    }

    const count = await this.collection.count();
    return { count, initialized: true };
  }

  /**
   * Clean shutdown.
   */
  async close(): Promise<void> {
    if (this.collection) {
      await this.collection.close();
      this.collection = null;
    }
    this.queryCache.clear();
  }
}
