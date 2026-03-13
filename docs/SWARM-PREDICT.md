# Swarm Predict: Testing & Handoff Guide

## What This Is

A new osModa skill (`skills/swarm-predict/SKILL.md`) that adds structured multi-perspective risk analysis before infrastructure changes. The agent role-plays 6-8 expert personas who debate a proposed change, surface risks from different angles, and produce a formal risk report with confidence scoring.

**Inspired by:** MiroFish (multi-agent swarm prediction engine) and PageIndex (vectorless document RAG). However, this is NOT a true multi-agent simulation — it's structured prompting within the existing agent. See "Honest Assessment" below.

## Files Changed

```
NEW   skills/swarm-predict/SKILL.md          — The skill definition (261 lines)
EDIT  nix/hosts/server.nix                   — Added PageIndex MCP server config
EDIT  CLAUDE.md                              — Updated skill count (17→18), added to lists
```

## How to Test

### Test 1: Skill File Validation

Verify the YAML frontmatter parses correctly and all referenced tools exist.

```bash
# Check frontmatter is valid YAML
head -20 skills/swarm-predict/SKILL.md

# Verify all 14 tools exist in the bridge
# These should all appear in packages/osmoda-bridge/index.ts:
grep -c 'system_health\|system_query\|journal_logs\|service_status\|file_read\|file_write\|shell_exec\|safe_switch_begin\|safe_switch_status\|safe_switch_commit\|safe_switch_rollback\|watcher_add\|teach_observe_action\|teach_knowledge_create' packages/osmoda-bridge/index.ts
# Expected: 14+ matches
```

### Test 2: Skill Discovery

On a running osModa server, verify the agent can see the new skill.

```bash
# SSH to a test server
ssh root@<server-ip>

# Check skill files are deployed in workspace
ls /root/.openclaw/workspace-osmoda/skills/swarm-predict/
# Expected: SKILL.md exists

# If not deployed yet, copy manually for testing:
mkdir -p /root/.openclaw/workspace-osmoda/skills/swarm-predict/
cp /opt/osmoda/skills/swarm-predict/SKILL.md /root/.openclaw/workspace-osmoda/skills/swarm-predict/
```

### Test 3: End-to-End via Chat

Connect to the agent via web chat or Telegram and test:

**Basic test prompt:**
```
I want to upgrade nginx from 1.24 to 1.26 on this server.
Run a swarm-predict risk analysis first.
```

**Expected behavior:**
1. Agent calls system_health(), system_query(), journal_logs() to gather context
2. Agent generates 6-8 personas with names, roles, and biases
3. Agent runs 3 rounds of debate with clear per-persona responses
4. Agent produces a risk report with:
   - Consensus risks (with severity)
   - Contested risks (with FOR/AGAINST)
   - Confidence score (0-100%)
   - Verdict: GO / NO-GO / GO WITH CONDITIONS
   - SafeSwitch plan (TTL, health checks)
5. If user approves, agent executes via safe_switch_begin()

**Failure modes to watch for:**
- Agent skips Phase 1 (no system data gathered) → bad
- All personas agree immediately (shallow analysis) → bad
- Report has no concrete metrics (vague risks like "something might break") → bad
- Agent proceeds without user approval → bad
- Confidence score doesn't match the actual debate (e.g., 95% but personas were split 4/4) → bad

### Test 4: Error Handling

Test Phase 1 failure:
```
Run a swarm-predict analysis for "deploy a new microservice"
but don't give me any details about what service.
```

**Expected:** Agent should ask for specifics or state it can't proceed without details. Should NOT generate generic personas and a meaningless report.

### Test 5: PageIndex MCP Integration

If PageIndex MCP is configured:

```
I have a migration guide PDF for PostgreSQL 16.
Index it with PageIndex, then run a swarm-predict analysis
for upgrading our PostgreSQL 15 to 16.
```

**Expected:** Agent uses PageIndex to index the document, then includes real migration notes in the situation briefing for Phase 1.

### Test 6: NixOS Config Validation

```bash
# Verify the nix config parses (requires nix)
nix eval .#nixosConfigurations.osmoda-server.config.services.osmoda.mcp.servers.pageindex.command
# Expected: "npx"

nix eval .#nixosConfigurations.osmoda-server.config.services.osmoda.mcp.servers.pageindex.args
# Expected: [ "-y" "@pageindex/mcp" ]
```

## Honest Assessment

### What this skill genuinely provides:
1. **Structured pre-flight checklist** — Forces data gathering before action
2. **Multiple-angle thinking** — 12 persona archetypes ensure security, cost, UX, ops perspectives are considered
3. **Formal risk scoring** — Confidence percentages and GO/NO-GO verdicts create decision discipline
4. **SafeSwitch integration** — Predictions connect to real deployment with auto-rollback (unique to osModa)
5. **Learning loop** — Outcomes stored via teach_knowledge_create for future reference

### What this skill does NOT provide:
1. **True emergent behavior** — One model role-playing personas is not the same as independent agents with separate memory (MiroFish uses OASIS with actual independent processes)
2. **Independent agent processes** — All personas share context, temperature, and the model's existing opinion. The model may unconsciously steer all personas toward its preferred answer
3. **Knowledge graph** — PageIndex does document RAG, not entity-relationship extraction. It's not a Zep Cloud replacement
4. **Statistical validity** — Research shows multi-agent debate doesn't consistently outperform self-consistency (just re-asking). The value is in the checklist structure, not the debate itself
5. **Thousand-agent simulation** — MiroFish can simulate 1M+ agents on social media. We do 6-8 personas in one context window

### Research backing:
- **Town Hall Debate Prompting (THDP, arXiv:2502.15725)**: +8-13% accuracy over single-pass on certain tasks using 5-7 personas in structured rounds. Our approach follows this pattern.
- **ICLR 2025 MAD benchmark**: Multi-agent debate doesn't reliably beat self-consistency. Our value is in the structured workflow, not the debate mechanic itself.
- **Agent Drift (arXiv:2601.04170)**: After ~73 interactions, agents drift significantly. Our 3-5 round format stays well within safe bounds.

### Bottom line:
This is a **well-structured decision-making framework** that leverages persona-based prompting. It's not swarm intelligence. Call it what it is: structured risk analysis with persona debate. The real innovation is connecting it to SafeSwitch for closed-loop predict→deploy→monitor→rollback→learn.

## For Future Development

To evolve this toward true swarm intelligence:

1. **Phase 1: Independent agent calls** — Instead of one prompt with all personas, make separate API calls per persona (different system prompts, maybe different temperatures). This gives actual independence.

2. **Phase 2: Real knowledge graph** — Build entity-relationship extraction into agentd's SQLite. Extract entities from system state (services, configs, dependencies) and their relationships. This replaces Zep Cloud with something local.

3. **Phase 3: OASIS-like simulation** — If we add a Python runtime to osModa (or a Rust simulation loop in a new `osmoda-swarm` daemon), we could run actual independent agent processes with separate memory and genuine emergent behavior.

4. **Phase 4: PageIndex for document grounding** — Already wired via MCP. Use it to index changelogs, runbooks, and post-mortems before simulations.

Each phase is independently valuable and doesn't require the others.
