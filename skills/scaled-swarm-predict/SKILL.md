---
name: scaled-swarm-predict
description: Large-scale social simulation with 50-200 demographically diverse AI personas debating on a simulated Twitter/Reddit board to predict outcomes
activation: manual
tools:
  - system_health
  - system_query
  - shell_exec
  - file_read
  - file_write
  - teach_knowledge_create
  - teach_observe_action
  - safe_switch_begin
  - safe_switch_status
  - safe_switch_commit
  - safe_switch_rollback
---

# Scaled Swarm Predict

Large-scale social simulation engine. Generates 50-200 demographically diverse AI personas, runs them through a simulated Twitter or Reddit board debating a topic, then analyzes emergent consensus, fault lines, and predictions.

**What this is:** A structured simulation where you generate a population of diverse personas matching real-world demographics, run 4-6 rounds of simulated social media discussion in a local SQLite database, then analyze position shifts and emergent consensus to make predictions.

**What this is NOT:** This is not MiroFish/OASIS (which runs independent agent processes). All personas are generated and role-played by a single model. The value comes from forcing diverse demographic perspectives through structured rounds of interaction — not from emergent multi-agent behavior. Think of it as a sophisticated polling simulation, not swarm intelligence.

**Cost per run:** ~$5-15 with Claude Sonnet (100 agents × 5 rounds = 500 generations). ~$25-60 with Opus. Each round is one large prompt, not 100 separate API calls.

## When to Use

- **Predict public reaction:** "How will Twitter react to this product launch?"
- **Election/poll modeling:** "What does a demographically representative sample think about X policy?"
- **Content testing:** "Will this announcement go viral or get ratio'd?"
- **Market sentiment:** "How will crypto twitter react to this protocol change?"
- **Risk assessment:** "What will the Reddit comments look like when we announce this pricing change?"
- **Brainstorming at scale:** "What arguments exist for/against X across different demographics?"

## Phase 1: Define the Simulation

### 1.1 — Get the Topic

Ask the user for:
1. **Topic/scenario** — What are we simulating discussion about?
2. **Platform** — Twitter (short-form, viral dynamics) or Reddit (long-form, threaded)
3. **Population size** — 50 (fast, cheap), 100 (balanced), 200 (thorough)
4. **Seed data** — Any documents, articles, data to ground the simulation

If seed data is provided, read it with `file_read()` and summarize key facts that personas will reference.

### 1.2 — Create the Board Database

Use `shell_exec()` to create a SQLite database for the simulation:

```bash
sqlite3 /var/lib/osmoda/swarm/sim_$(date +%s).db "
CREATE TABLE personas (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  age INTEGER,
  gender TEXT,
  location TEXT,
  occupation TEXT,
  political_lean TEXT,
  income_bracket TEXT,
  education TEXT,
  interests TEXT,
  personality TEXT,
  initial_position TEXT,
  bio TEXT
);

CREATE TABLE posts (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  round INTEGER NOT NULL,
  persona_id INTEGER NOT NULL,
  reply_to INTEGER,
  content TEXT NOT NULL,
  position TEXT,
  sentiment REAL,
  likes INTEGER DEFAULT 0,
  retweets INTEGER DEFAULT 0,
  FOREIGN KEY (persona_id) REFERENCES personas(id),
  FOREIGN KEY (reply_to) REFERENCES posts(id)
);

CREATE TABLE position_shifts (
  persona_id INTEGER NOT NULL,
  round INTEGER NOT NULL,
  position_before TEXT,
  position_after TEXT,
  reason TEXT,
  PRIMARY KEY (persona_id, round),
  FOREIGN KEY (persona_id) REFERENCES personas(id)
);

CREATE TABLE analysis (
  round INTEGER PRIMARY KEY,
  summary TEXT,
  top_arguments_for TEXT,
  top_arguments_against TEXT,
  consensus_pct REAL,
  viral_posts TEXT
);
"
```

Store the database path for all subsequent phases.

## Phase 2: Generate Population

### 2.1 — Define Demographics

Match the population to the scenario. For US topics, use approximate census distributions:

```
Age:        18-24 (15%), 25-34 (20%), 35-44 (18%), 45-54 (16%), 55-64 (15%), 65+ (16%)
Gender:     Male (49%), Female (50%), Non-binary (1%)
Education:  High school (27%), Some college (20%), Bachelor's (22%), Graduate (13%), Trade/other (18%)
Political:  Liberal (25%), Moderate-left (15%), Center (20%), Moderate-right (15%), Conservative (25%)
Income:     <$30k (20%), $30-60k (25%), $60-100k (25%), $100-150k (18%), $150k+ (12%)
```

For tech/crypto topics, skew younger, more male, higher education. For local issues, adjust by region. Always tell the user what distribution you're using and why.

### 2.2 — Generate Personas in Batches

Generate personas in batches of 20-25 to stay within context limits. Each batch prompt:

```
Generate [N] realistic social media personas for a [Twitter/Reddit] simulation about "[topic]".

Demographics for this batch: [specify age/gender/political/income distribution for this batch]

For each persona, output JSON:
{
  "name": "realistic full name matching demographics",
  "age": N,
  "gender": "...",
  "location": "city, state",
  "occupation": "specific job title",
  "political_lean": "liberal|moderate-left|center|moderate-right|conservative",
  "income_bracket": "$X-Y",
  "education": "specific degree or level",
  "interests": "3-5 specific interests",
  "personality": "one sentence: how they argue online (aggressive, thoughtful, sarcastic, lurker-who-snaps, etc.)",
  "initial_position": "support|oppose|undecided|unaware",
  "bio": "their Twitter/Reddit bio, 1-2 sentences, authentic voice"
}

Make them feel REAL — not stereotypes. A conservative plumber can care about climate.
A liberal professor can oppose regulation. Surprise me with 20% of them.
```

Insert each batch into the `personas` table via `shell_exec()` with sqlite3.

### 2.3 — Validate Distribution

After all batches, verify the demographics match targets:

```bash
sqlite3 $DB_PATH "
SELECT political_lean, COUNT(*), ROUND(COUNT(*)*100.0/(SELECT COUNT(*) FROM personas),1) as pct
FROM personas GROUP BY political_lean ORDER BY pct DESC;
"
```

Report the actual distribution to the user. If any category is off by >5%, generate corrective personas.

## Phase 3: Run Simulation Rounds

Run 4-6 rounds. Each round is ONE prompt that generates all persona responses for that round.

### Round 1 — First Reactions (all personas post)

```
You are simulating a [Twitter/Reddit] board. [N] users are reacting to this:

"[TOPIC/ANNOUNCEMENT — paste seed data summary here]"

Here are the users (JSON array of all personas).

Generate a [Twitter post (max 280 chars) / Reddit comment (2-4 sentences)] for EACH persona.
Stay in character — match their age, education, politics, personality, and online voice.

For each persona output:
- persona_id: [N]
- content: "[their post]"
- position: "support|oppose|mixed|joke|question"
- sentiment: [-1.0 to 1.0]

Some personas should:
- Post memes or jokes (15%)
- Ask genuine questions (10%)
- Share personal anecdotes (10%)
- Make strong arguments for/against (40%)
- Just react emotionally (15%)
- Stay silent this round (10% — not everyone posts immediately)

Do NOT make everyone articulate. Some posts should be typos, slang, ALL CAPS, one-word reactions.
```

Parse the output and INSERT into the `posts` table.

### Round 2 — Replies and Reactions

```
Round 1 posts are below. Now generate Round 2: replies and reactions.

[Paste Round 1 posts — all of them, with persona names and IDs]

Rules for Round 2:
- Each persona reads 5-10 random posts from Round 1
- They reply to 1-3 posts (set reply_to = that post's ID)
- They may also make a new top-level post
- 20% of personas shift position based on arguments they read
- Record any position shift in their response

For position shifts, also output:
- position_before
- position_after
- reason: "one sentence — what convinced them"

Viral dynamics: The top 3 posts from Round 1 (by argument strength) get more replies.
Controversial posts get more replies than consensus posts.
```

Parse and INSERT posts + position_shifts.

### Round 3 — Debate Heats Up

```
Rounds 1-2 are below. Generate Round 3: the debate intensifies.

[Paste all posts from rounds 1-2 with context]

Round 3 dynamics:
- Tribal clusters form: personas start agreeing with similar demographics
- Counter-arguments appear: personas cite specific earlier posts to rebut
- Information spreads: facts from seed data appear in arguments (with varying accuracy)
- 10% of personas get heated (personal attacks, sarcasm, ratio attempts)
- 15% more position shifts
- Lurkers from Round 1 now engage (the silent 10%)
- 5% of personas disengage ("this is pointless" / "touch grass")
```

### Round 4 — Consolidation

```
Full thread so far (Rounds 1-3). Generate Round 4: positions solidify.

Round 4 dynamics:
- Most personas have firm positions now
- Summary posts appear ("the real issue is...")
- Final position shifts (5-10% max — late deciders)
- Thread fatigue: 30% of personas stop posting
- Quality arguments bubble up, low-effort posts sink
- Some personas change the topic or go meta ("why are we even arguing about this")
```

### Optional Round 5 — Next Day Follow-up

```
24 hours later. The thread has cooled. Generate Round 5.

Round 5 dynamics:
- Only 30-40% of personas return
- Tone shifts: more reflective, less combative
- New information may have emerged (you can introduce one new fact)
- "I thought about it and..." posts (final position shifts)
- Someone writes a long "thread" summarizing both sides
```

### Optional Round 6 — External Event

Only use if the simulation benefits from a disruption:

```
BREAKING: [New information related to the topic]

How do personas react? Generate Round 6.
- 60% of personas react to the new info
- Position shifts may be dramatic
- Previous arguments get re-evaluated
```

## Phase 4: Analyze Results

### 4.1 — Quantitative Analysis

Run these queries against the database:

```bash
# Final position distribution
sqlite3 $DB_PATH "
SELECT position, COUNT(*) as count,
  ROUND(COUNT(*)*100.0/(SELECT COUNT(*) FROM posts WHERE round=(SELECT MAX(round) FROM posts)),1) as pct
FROM posts WHERE round = (SELECT MAX(round) FROM posts)
GROUP BY position ORDER BY count DESC;
"

# Position shifts over time
sqlite3 $DB_PATH "
SELECT round, position_after, COUNT(*) as shifts
FROM position_shifts
GROUP BY round, position_after
ORDER BY round, shifts DESC;
"

# Most influential posts (most replies received)
sqlite3 $DB_PATH "
SELECT p.id, per.name, p.content, COUNT(r.id) as reply_count
FROM posts p
JOIN personas per ON p.persona_id = per.id
LEFT JOIN posts r ON r.reply_to = p.id
GROUP BY p.id
ORDER BY reply_count DESC
LIMIT 10;
"

# Sentiment by demographic
sqlite3 $DB_PATH "
SELECT per.political_lean, ROUND(AVG(p.sentiment),2) as avg_sentiment
FROM posts p JOIN personas per ON p.persona_id = per.id
WHERE p.round = (SELECT MAX(round) FROM posts)
GROUP BY per.political_lean;
"

# Age group sentiment
sqlite3 $DB_PATH "
SELECT
  CASE WHEN per.age < 25 THEN '18-24'
       WHEN per.age < 35 THEN '25-34'
       WHEN per.age < 45 THEN '35-44'
       WHEN per.age < 55 THEN '45-54'
       WHEN per.age < 65 THEN '55-64'
       ELSE '65+' END as age_group,
  ROUND(AVG(p.sentiment),2) as avg_sentiment,
  COUNT(DISTINCT per.id) as persona_count
FROM posts p JOIN personas per ON p.persona_id = per.id
WHERE p.round = (SELECT MAX(round) FROM posts)
GROUP BY age_group ORDER BY age_group;
"
```

### 4.2 — Qualitative Analysis

After running the queries, analyze the full simulation transcript:

```
Read the full simulation (all rounds, all posts, all position shifts).

Produce this analysis:

1. PREDICTION: What does this simulated population predict/prefer? (1 sentence)
2. CONFIDENCE: How strong is the consensus? (percentage + explanation)
3. FAULT LINES: Where does the population split? (demographics, not just opinion)
4. VIRAL ARGUMENTS: The 3 arguments that changed the most minds (quote them)
5. SURPRISING FINDINGS: What positions appeared that you didn't expect?
6. DEMOGRAPHIC BREAKDOWN: Support/oppose by age, gender, political lean, income
7. SIMULATION HEALTH: Did the personas feel real? Were any rounds shallow?
```

### 4.3 — Produce Report

Write the final report to a file:

```bash
file_write({
  path: "/var/lib/osmoda/swarm/report_[timestamp].md",
  content: "[full report markdown]"
})
```

Report format:

```markdown
# Scaled Swarm Prediction Report

## Topic
[What was simulated]

## Population
- Size: [N] personas
- Platform: [Twitter/Reddit]
- Rounds: [N]
- Demographic match: [% accuracy to target distribution]

## Prediction
[One clear sentence: what the simulated population predicts/prefers]

## Confidence: [X%]
[Why this confidence level — based on consensus strength, not gut feeling]

## Position Distribution (Final Round)
| Position | Count | % |
|----------|-------|---|
| Support  | N     | X |
| Oppose   | N     | X |
| Mixed    | N     | X |
| Other    | N     | X |

## Demographic Breakdown
[Table: position by age group, political lean, gender, income]

## Key Fault Lines
1. [Demographic split #1 — e.g., "Under-35s support 3:1, over-55s oppose 2:1"]
2. [Demographic split #2]

## Most Influential Arguments
1. "[Quote]" — by [persona name/type], shifted [N] positions
2. "[Quote]"
3. "[Quote]"

## Surprising Findings
- [Thing that broke expectations]

## Position Shifts Over Time
| Round | Support→Oppose | Oppose→Support | Newly Engaged |
|-------|---------------|----------------|---------------|
| 1     | -             | -              | N             |
| 2     | N             | N              | N             |
| ...   |               |                |               |

## Simulation Health
- Persona authenticity: [HIGH/MED/LOW — did they feel real?]
- Debate quality: [Did arguments evolve or repeat?]
- Known bias: [Single model role-playing all personas — positions may cluster]

## Raw Data
Database: [path to SQLite file]
```

### 4.4 — Store for Learning

```
teach_knowledge_create({
  title: "Swarm prediction: [topic]",
  category: "prediction",
  content: "[report summary + prediction + confidence]",
  tags: ["scaled-swarm-predict", "[topic-tag]", "[platform]"]
})
```

## Phase 5: Act on Prediction (Optional)

If the prediction is actionable and the user approves:

```
safe_switch_begin({
  plan: "Action based on swarm prediction: [what]",
  ttl_secs: 1800,
  health_checks: [from prediction context]
})
```

Execute the action, monitor, commit or rollback based on results.

After the action completes, update the knowledge entry with the actual outcome vs prediction. This is the most valuable data — where the simulation was right and where it was wrong.

## Limitations (Be Honest)

1. **Single model bias:** All 200 personas are generated by one model. The model has its own biases that leak into every persona. Conservative personas written by Claude may not sound like actual conservatives on Twitter.

2. **No real network effects:** Real Twitter has follower graphs, algorithmic amplification, bots, quote-tweet dunking culture. This simulation has none of that. Viral dynamics are approximated, not modeled.

3. **Context window limits:** 200 personas × 5 rounds = a lot of text. Later rounds may lose early context. Use the SQLite database as the source of truth, not the model's memory.

4. **Not statistically rigorous:** This is a thought experiment at scale, not a scientific poll. The demographic matching helps, but 100 AI personas are not 100 real people. Treat confidence scores as relative, not absolute.

5. **Cost scales linearly:** More personas or more rounds = more tokens = more cost. The sweet spot is 100 personas × 5 rounds (~$8-12 with Sonnet).

## Example: "How will Twitter react to osModa launching at $14.99/month?"

**Phase 2 output (sample personas):**
- @devops_dan, 38, SRE at fintech, moderate, $120k — "I pay $20/mo for a Hetzner VPS. Why would I pay for an AI wrapper?"
- @sarahbuilds, 26, indie hacker, progressive, $45k — "Wait this actually manages my server? I hate DevOps. Take my money."
- @crypto_mike, 31, DeFi trader, libertarian, $200k+ — "Does it have wallet support? Can it run validators?"
- @linux_grandpa, 62, retired sysadmin, conservative, pension — "Another AI grift. Learn bash."

**Phase 3 results (after 5 rounds):**
- Support: 42%, Oppose: 31%, Mixed: 18%, Jokes: 9%
- Fault line: Devs with sysadmin skills oppose (they don't need it). Non-technical builders support (they hate server management).
- Viral argument: "It's not replacing sysadmins, it's giving non-technical people sysadmin superpowers" — shifted 12 positions.
- Prediction: Positive reception among indie hackers and AI-tool-users. Skepticism from traditional DevOps. Price point acceptable for target audience.
