/**
 * Session management — maps user/channel to Claude Code SDK session IDs.
 * Sessions expire after 30 minutes of inactivity.
 */

const SESSION_TIMEOUT_MS = 30 * 60 * 1000;

export interface Session {
  id: string;
  agentId: string;
  claudeSessionId?: string;
  lastActivity: number;
  userId: string;
  channel: string;
}

export class SessionStore {
  private sessions = new Map<string, Session>();

  getOrCreate(userId: string, channel: string, agentId: string): Session {
    const key = `${channel}:${userId}`;
    const existing = this.sessions.get(key);
    const now = Date.now();

    if (existing && now - existing.lastActivity < SESSION_TIMEOUT_MS) {
      existing.lastActivity = now;
      return existing;
    }

    // Expired or new — create fresh session
    const session: Session = {
      id: `sess-${now}-${Math.random().toString(36).slice(2, 8)}`,
      agentId,
      claudeSessionId: undefined,
      lastActivity: now,
      userId,
      channel,
    };
    this.sessions.set(key, session);
    return session;
  }

  updateClaudeSession(userId: string, channel: string, claudeSessionId: string): void {
    const key = `${channel}:${userId}`;
    const session = this.sessions.get(key);
    if (session) {
      session.claudeSessionId = claudeSessionId;
    }
  }

  /** Remove expired sessions */
  prune(): number {
    const now = Date.now();
    let pruned = 0;
    for (const [key, session] of this.sessions) {
      if (now - session.lastActivity > SESSION_TIMEOUT_MS) {
        this.sessions.delete(key);
        pruned++;
      }
    }
    return pruned;
  }

  get size(): number {
    return this.sessions.size;
  }
}
