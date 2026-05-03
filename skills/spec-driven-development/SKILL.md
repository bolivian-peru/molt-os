---
name: spec-driven-development
description: >
  Build software via spec-driven development (github/spec-kit). Whenever the
  user asks for a feature larger than a one-line tweak, scaffold a spec-kit
  project, capture WHAT + WHY, declare tech stack, break into tasks, then
  iterate the implementation until tests pass.
tools:
  - spec_kit_init
  - spec_kit_run
  - file_read
  - file_write
  - directory_list
  - shell_exec
  - memory_recall
  - memory_store
activation: auto
---

# Spec-Driven Development Skill

You ship software through GitHub's [spec-kit](https://github.com/github/spec-kit) (92K stars, MIT, baked into every osModa spawn). The workflow makes you a **closed loop**: humans declare WHAT and WHY; you generate plan, tasks, code; tests are the inner-loop signal of success; the human is the outer-loop reviewer.

## When to invoke spec-kit

**Use spec-kit when**:
- The user asks for a feature implementation: "build me X", "add Y", "implement Z", "rewrite this so it does W"
- The work spans more than ~5 file edits
- Tests will exist that you can iterate against
- The user wants a reproducible artifact trail (specs, plans, tasks)

**Skip spec-kit when**:
- One-line fix or typo
- Operational task: use `system-monitor`, `self-healing`, `app-deployer` instead
- Exploratory hack: free-form chat
- The user explicitly says "just edit the file" / "don't write a spec"

## The 8-step workflow

| # | Skill | Purpose |
|---|---|---|
| 1 | `speckit-constitution` | Project governance: code style, test policy, language choice |
| 2 | `speckit-specify` | What & why (no tech stack yet). User stories with priorities. |
| 3 | `speckit-clarify` *(optional)* | Ask user for gaps before locking the plan |
| 4 | `speckit-plan` | Tech stack + architecture choices |
| 5 | `speckit-tasks` | Ordered actionable task list |
| 6 | `speckit-analyze` *(optional)* | Cross-artifact consistency check |
| 7 | `speckit-checklist` *(optional)* | Quality gates before implement |
| 8 | `speckit-implement` | Generate code, run tests, iterate until green |

## How to call from your tool box

The two MCP tools `spec_kit_init` + `spec_kit_run` wrap the spec-kit workflow. Use them — don't shell out to `specify` directly. Tool calls are audit-ledgered; raw shell isn't.

### Step 1 — scaffold

```
spec_kit_init({
  project_name: "csv-exporter",
  integration: "claude",
  constitution_seed: "Python 3.12+, pytest for tests, no async, single binary"
})
```

Returns `{ project_path: "/workspace/csv-exporter", skills: [9 speckit-*], next_action }`. The 9 skills become available as `/speckit-*` slash commands inside that project.

### Step 2 — set governance

```
spec_kit_run({
  project_path: "/workspace/csv-exporter",
  command: "constitution",
  prompt: "Python 3.12+, pytest, no async runtime, all functions have docstrings, max 200 LOC per module"
})
```

Writes `/workspace/csv-exporter/memory/constitution.md`. Read the user's `.specify/.constitution-seed.md` first if it exists — they may have pre-staged principles.

### Step 3 — capture WHAT and WHY

```
spec_kit_run({
  project_path: ".../csv-exporter",
  command: "specify",
  prompt: "<user's exact feature request, expanded with their goals>"
})
```

Resist the urge to declare a tech stack here — that's step 4. Specify is purely product-level.

### Step 4 — clarify (only if specify left gaps)

```
spec_kit_run({ command: "clarify", prompt: "" })
```

This is interactive in the user-facing flow. From your loop: skip unless `specify` output mentions ambiguities.

### Step 5 — declare the plan

```
spec_kit_run({
  command: "plan",
  prompt: "Python 3.12, click for CLI, pandas for parsing, pytest for tests, single-file binary via shiv"
})
```

Writes `/specs/<feature>/plan.md`.

### Step 6 — break into tasks

```
spec_kit_run({ command: "tasks", prompt: "" })
```

Writes `/specs/<feature>/tasks.md` with numbered ordered tasks.

### Step 7 — implement

```
spec_kit_run({
  command: "implement",
  prompt: "",
  timeout_seconds: 1800   // give it 30 min for non-trivial features
})
```

This is the long step. The agent reads `tasks.md`, generates code, runs tests, iterates. **Do not interrupt unless the user explicitly aborts.** Token usage will be high — that's by design (token-max philosophy).

## Common pitfalls

- **Don't run `implement` before `plan`** — the implement step reads `plan.md` for the tech stack. Without it, the agent guesses (often wrong).
- **Don't write the spec FOR the user.** Capture their exact words for the WHAT/WHY. If their description is too vague to act on, run `clarify` instead of inventing requirements.
- **Don't skip `constitution` for multi-feature projects.** It's the cross-feature governance contract. Without it, feature 2 will violate decisions you implicitly made in feature 1.
- **Don't run two `spec_kit_run`s in parallel for the same project.** They contend for the same `.specify/` state. Sequence them.
- **Don't forget the audit trail.** Every `spec_kit_run` is hash-chained in agentd's ledger. Querying `agentctl events --type spec-kit` shows the full project history. Show this to the user when they ask "what did you do?"

## Telling the user about progress

After `spec_kit_init`: "Project scaffolded at /workspace/csv-exporter. Now capturing requirements."

After `specify`: "Spec captured. Read /workspace/csv-exporter/specs/0001-*/spec.md and review the user stories. Want me to clarify anything before locking the plan?"

After `plan`: "Tech stack locked: Python 3.12 + click + pytest. Tasks coming next."

During `implement`: report at task boundaries. Don't stream every line of compile output.

After `implement` (tests green): "Implementation complete. Tests pass. Specs and code are at /workspace/csv-exporter/. Next: review the diff (`git diff` if you initialized git) or run the binary."

If `implement` fails after N iterations: stop, report which tests failed, ask the user. Don't burn tokens flailing — they're cheap, but goodwill isn't.

## Reference templates

The pre-deployed templates live at `/var/lib/osmoda/templates/spec-kit/`. Useful when:
- A user asks "what does a spec look like?" — show them `spec-template.md`
- You want to reference the canonical constitution structure — `constitution-template.md`
- You're debugging why a `speckit-*` skill output looks wrong — compare against the template

## Audit + visibility

Every `spec_kit_*` tool call writes to agentd's hash-chained ledger:
```
agentctl events --type spec-kit --limit 20
```

External integrators can query the per-server `/api/v1/spec-kit/projects` endpoint (see Phase 7-LITE) to discover all spec-kit projects on a given osModa server. This makes the substrate **queryable** in the YC sense — every action produces an artifact the intelligence at the center of the company can learn from.

## Why this skill exists

osModa's positioning: *GitHub gives you the workflow. osModa gives you the workstation.* Spec-driven development is the canonical AI-coding-agent workflow as of 2026 — joining it is how we live the "software factories" YC principle structurally, not aspirationally. Use it for non-trivial work; the user is opting into a real software factory each time.
