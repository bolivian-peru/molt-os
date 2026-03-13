---
name: swarm-predict
description: Multi-perspective risk analysis using structured persona debate before deploying changes
activation: auto
tools:
  - system_health
  - system_query
  - journal_logs
  - service_status
  - file_read
  - file_write
  - shell_exec
  - safe_switch_begin
  - safe_switch_status
  - safe_switch_commit
  - safe_switch_rollback
  - watcher_add
  - teach_observe_action
  - teach_knowledge_create
---

# Swarm Predict

Structured multi-perspective risk analysis before acting on infrastructure changes. Uses persona-based debate to surface risks from different viewpoints, then deploys via SafeSwitch with auto-rollback.

**What this is:** A structured prompting technique where you role-play 6-8 expert personas debating a proposed change. It forces consideration of multiple angles (security, reliability, cost, UX) before committing. Think of it as a pre-flight checklist, not a crystal ball.

**What this is NOT:** This is not true multi-agent simulation (like MiroFish/OASIS with independent agent processes). All personas share one context window and one model. The value comes from structured thinking and the checklist effect, not from emergent behavior.

## When to Use

- Before deploying infrastructure changes ("What if we switch to nginx?")
- Before system upgrades ("Will upgrading PostgreSQL break anything?")
- Incident response ("What's the safest recovery path?")
- Any change where you want a second opinion but don't have a team to consult

## Workflow

### Phase 1: Gather Context

Collect real system state. The analysis is only as good as the data it's grounded in.

```
1. system_health() → CPU, RAM, disk, load, uptime
2. system_query({ query: "services" }) → running services
3. journal_logs({ unit: "relevant-service", lines: 50 }) → recent activity
4. file_read({ path: "/relevant/config/file" }) → current config
```

**Minimum data checklist** — do NOT proceed without:
- [ ] system_health returned CPU/RAM/disk numbers
- [ ] At least one service query succeeded
- [ ] The proposed change is specific (not vague like "improve performance")

If data collection fails, tell the user: "Cannot run analysis without baseline system state. Please provide context manually or fix the service queries."

Build a **situation briefing** — a concise paragraph with:
- Current system state (concrete numbers, not "healthy")
- The exact proposed change
- Known constraints or dependencies

### Phase 2: Select Personas

Pick 6-8 from this table. Choose archetypes relevant to the change — don't use all 12.

| Archetype | Optimizes for | Blind spot |
|---|---|---|
| **Ops Engineer** | Reliability, uptime, monitoring | Over-conservative, blocks progress |
| **Security Analyst** | Attack surface, CVEs, access control | Paranoid, sees threats everywhere |
| **Performance Engineer** | Latency, throughput, efficiency | Optimistic about gains, ignores stability |
| **End User** | Response time, zero disruption | No technical context, just wants it to work |
| **Cost Analyst** | $/hour, resource waste | Penny-wise, pound-foolish |
| **Junior Dev** | Simplicity, documentation | Asks naive questions that reveal assumptions |
| **Chaos Engineer** | Failure modes, blast radius | Adversarial by nature, can over-index on unlikely scenarios |
| **Compliance Officer** | Audit trails, regulations | Blocks anything undocumented |
| **Database Admin** | Data integrity, migrations, backups | Extremely cautious, can stall decisions |
| **Network Engineer** | DNS, routing, firewall, latency | Hyper-focused on connectivity edge cases |
| **SRE Lead** | SLOs, error budgets, rollback plans | Balanced but demands extensive rollback planning |
| **Product Manager** | Timelines, feature velocity | Underestimates risk, wants speed |

For each selected persona, define:
```
Name: [Realistic name]
Role: [Title]
Optimizes for: [1 sentence]
Blind spot: [1 sentence]
```

### Phase 3: Run Debate (3 rounds mandatory, 2 optional)

Each round is a single prompt containing the situation briefing, all persona definitions, full prior discussion, and the round instruction. Output each persona's response labeled by name.

**Round 1 — Initial Reactions:**
```
Given the situation and your role, state:
1. Your biggest concern about this change
2. One risk others might miss
3. Your initial position (support / oppose / conditional)
Each persona: 2-3 sentences. Be specific — cite actual services, configs, versions.
```

**Round 2 — Challenge:**
```
Read Round 1. Now:
1. Name one thing another persona said that you disagree with, and why
2. Name one thing another persona said that changed your thinking
3. Propose one concrete mitigation for the top risk
Each persona: 3-4 sentences. Reference others by name.
```

**Round 3 — Final Position:**
```
Read the full discussion. State:
1. Your final recommendation: GO / NO-GO / GO WITH CONDITIONS
2. The single most important condition (if GO WITH CONDITIONS)
3. One sentence: what breaks first if this goes wrong?
Each persona: 2-3 sentences. No hedging — commit to a position.
```

**Optional Round 4 — Red Team (use for high-stakes changes):**
```
The change IS deployed. Try to break it.
1. Most likely failure in the first hour
2. Sneaky failure that appears after a week
Each persona: 1-2 sentences. Be adversarial.
```

**Optional Round 5 — Deployment Plan (use when proceeding):**
```
Draft the deployment plan as a group:
1. Pre-flight checks (what to verify before starting)
2. Execution order (step by step)
3. Rollback trigger (what specific metric/event means abort)
4. Health checks during and after
```

### Phase 4: Score and Report

Count positions from Round 3 and apply these rules:

| Outcome | Threshold | Confidence |
|---|---|---|
| **GO** | All personas support or support-with-conditions | 85-95% |
| **GO WITH CONDITIONS** | 5+ of 8 support, dissenters' concerns addressable | 65-85% |
| **NEEDS MORE DATA** | 4/4 split or concerns based on unknown system state | 40-65% |
| **NO-GO** | 5+ of 8 oppose | Recommend delay |

**Adjust confidence down** for:
- Vague risks ("something might break") → -10%
- Missing system data (Phase 1 gaps) → -15%
- All personas suspiciously agree (likely shallow analysis) → -10%

Produce this report:

```markdown
## Risk Analysis Report

### Change
[What was evaluated — 1 sentence]

### System Context
[Key metrics from Phase 1]

### Consensus Risks
- [Risk everyone agrees on] — Severity: HIGH/MED/LOW
- [Another consensus risk] — Severity: HIGH/MED/LOW

### Contested Risks
- [Risk with disagreement]
  - Concerned: [Who and why]
  - Dismisses: [Who and why]

### Verdict: [GO / GO WITH CONDITIONS / NEEDS MORE DATA / NO-GO]
Confidence: [X%]

### Conditions (if applicable)
1. [Specific, actionable condition]
2. [Another condition]

### SafeSwitch Plan
- Pre-flight: [checks before starting]
- TTL: [seconds before auto-rollback]
- Health checks: [what to monitor]
- Rollback trigger: [what constitutes failure]
```

### Phase 5: Execute (only if GO or GO WITH CONDITIONS, and user approves)

```
1. safe_switch_begin({
     plan: "[change description]",
     ttl_secs: [from report],
     health_checks: [from report]
   })

2. Execute the change using appropriate tools

3. watcher_add({
     name: "post-change-monitor",
     check: { type: [from report] },
     interval_secs: 30,
     actions: ["notify", "rollback"]
   })

4. Monitor for TTL duration:
   - If health checks pass → safe_switch_commit()
   - If any check fails → safe_switch_rollback()

5. Record outcome:
   teach_knowledge_create({
     title: "Risk analysis: [change]",
     category: "prediction",
     content: "[report + actual outcome + which risks materialized]",
     tags: ["swarm-predict", "[go/nogo]", "[success/rollback]"]
   })
```

### If Something Goes Wrong

- **Phase 1 fails (can't read system state):** STOP. Tell user. Don't guess.
- **All personas agree too easily:** Re-run with a Chaos Engineer and Junior Dev persona forced in. Unanimous agreement on infrastructure changes is suspicious.
- **Confidence < 40%:** Do NOT proceed. Ask user for more context or simplify the change.
- **SafeSwitch rollback triggers:** Record what happened. The mismatch between prediction and reality is the most valuable data for future analyses.

## Example: "Should we upgrade Node.js 18 → 22?"

**Phase 1 — Context:**
```
CPU: 45%, RAM: 62%, Disk: 60%. Uptime: 34 days.
Services: node (18.19, port 3000, 200 req/s, p99 45ms), postgresql-15, nginx.
Config: package.json engines field says ">=16", 47 dependencies.
```

**Phase 2 — Personas:** Sarah (Ops), Viktor (Security), Priya (Performance), Alex (User), James (Junior Dev), Mei (SRE Lead)

**Phase 3 — 3 rounds produce:**
- James: "Do our 47 npm packages support Node 22? `engines` says >=16 but that's package.json, not every dep."
- Viktor: "Node 18 EOL April 2025. We're already past it. Security risk of NOT upgrading."
- Sarah: "Zero-downtime requires keeping 18 binary for instant rollback."
- Mei: "SafeSwitch with 15-min TTL. Health check: HTTP 200 on /health + p99 < 100ms."

**Phase 4 — Report:**
```
Verdict: GO WITH CONDITIONS (Confidence: 78%)
Conditions:
1. Run npm ls --all and check for Node 22 incompatibilities first
2. Deploy during 2-4 AM low-traffic window
3. Keep Node 18 binary at /usr/local/bin/node18 for rollback
SafeSwitch: TTL 900s, check HTTP 200 on :3000/health every 30s
```

**Phase 5 — Deploy via SafeSwitch. Auto-rollback if /health fails within 15 min.**

## Integration with PageIndex MCP

If PageIndex MCP is available, use it to index relevant documentation before Phase 1:
- Upstream changelogs/migration guides for the software being changed
- Internal runbooks or post-mortems from similar changes
- Vendor documentation for affected services

This gives personas access to real documentation context instead of relying on training data alone.
