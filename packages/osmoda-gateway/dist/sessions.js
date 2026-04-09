/**
 * Session management — maps user/channel to Claude Code SDK session IDs.
 * Sessions expire after 30 minutes of inactivity.
 */
const SESSION_TIMEOUT_MS = 30 * 60 * 1000;
export class SessionStore {
    sessions = new Map();
    getOrCreate(userId, channel, agentId) {
        const key = `${channel}:${userId}`;
        const existing = this.sessions.get(key);
        const now = Date.now();
        if (existing && now - existing.lastActivity < SESSION_TIMEOUT_MS) {
            existing.lastActivity = now;
            return existing;
        }
        // Expired or new — create fresh session
        const session = {
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
    updateClaudeSession(userId, channel, claudeSessionId) {
        const key = `${channel}:${userId}`;
        const session = this.sessions.get(key);
        if (session) {
            session.claudeSessionId = claudeSessionId;
        }
    }
    /** Remove expired sessions */
    prune() {
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
    get size() {
        return this.sessions.size;
    }
}
