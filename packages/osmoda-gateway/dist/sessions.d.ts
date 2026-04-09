/**
 * Session management — maps user/channel to Claude Code SDK session IDs.
 * Sessions expire after 30 minutes of inactivity.
 */
export interface Session {
    id: string;
    agentId: string;
    claudeSessionId?: string;
    lastActivity: number;
    userId: string;
    channel: string;
}
export declare class SessionStore {
    private sessions;
    getOrCreate(userId: string, channel: string, agentId: string): Session;
    updateClaudeSession(userId: string, channel: string, claudeSessionId: string): void;
    /** Remove expired sessions */
    prune(): number;
    get size(): number;
}
