# Procedural Memory (TODO)

## What is Procedural Memory?

In cognitive science, procedural memory is "knowing how" — skills, habits, and learned behaviors that operate automatically. Unlike episodic memory (what happened) or semantic memory (what I know), procedural memory governs *how to act*.

For a cyber waifu, this translates to **interaction patterns, behavioral rules, and communication preferences** learned from conversation:

| Category | Example |
|---|---|
| Communication style | "User prefers code examples before explanations" |
| Behavioral boundary | "Don't bring up topic X, it upsets user" |
| Conditional response | "When user is stressed, keep messages shorter" |
| Interaction ritual | "User says 'oyasumi' → respond with specific goodnight ritual" |
| Preference | "Always use dark mode examples in code screenshots" |

## Design: Reuse Semantic Memory (MVP)

### Why Not a Separate System?

Procedural knowledge is structurally similar to semantic facts — it's long-term knowledge distilled from episodes. The difference is in *what it describes*:

| | Semantic Fact | Procedural Rule |
|---|---|---|
| Describes | State of the world | How to behave |
| Example | "User lives in Tokyo" | "When user is upset, be gentle" |
| Triple | `(user, lives_in, Tokyo)` | `(assistant, should_when_upset, "be gentle")` |

For MVP, procedural rules are stored as **semantic facts with `subject = "assistant"`** and behavioral predicates. No new table, no new extraction pipeline.

### Procedural Predicates

The LLM extraction prompt already supports free-form predicates. For procedural rules, we recommend:

```
Procedural predicates (for behavioral rules):
- should, should_not, should_when_[context]
- responds_to_[trigger]_with
- prefers_to, avoids
```

### Examples

```
("assistant", "should", "use Rust examples when explaining code")
("assistant", "should_not", "mention user's ex")
("assistant", "should_when_user_upset", "be gentle and use shorter messages")
("assistant", "responds_to_oyasumi_with", "おやすみ、いい夢見てね 🌙")
("we", "have_routine", "Monday morning check-in about the weekend")
```

> [!NOTE]
> Some of these overlap with "we" facts in semantic memory. That's fine — the boundary between "what we do" (semantic) and "how I should act" (procedural) is naturally fuzzy. Both are retrieved together.

### Retrieval and Presentation

Procedural facts are retrieved alongside semantic facts via the same hybrid search. In the tool result, they can be presented under a separate heading:

```markdown
## Known Facts
- User lives in Tokyo (sources: 2 conversations)
- User likes Rust (sources: 3 conversations)

## Behavioral Guidelines
- When user is upset, be gentle and brief (sources: 1 conversation)
- Always use Rust examples when explaining code (sources: 2 conversations)

## Episodic Memories
...
```

The separation is done at presentation time by filtering on `subject = "assistant"` + procedural predicates, not at storage time.

### Extraction

The existing Semantic Extraction Job handles procedural rules — just extend the LLM prompt:

```
Extract lasting knowledge from this conversation segment.

Categories to extract:
1. Facts about the user (preferences, personal info, relationships)
2. Facts about the relationship ("we" subject)
3. Behavioral rules for the assistant:
   - Communication preferences the user has expressed
   - Topics to avoid or emphasize
   - Interaction patterns and rituals
   - Conditional behavior (when X happens, do Y)

For behavioral rules, use subject = "assistant" with predicates like:
should, should_not, should_when_[context], responds_to_[trigger]_with
```

## When to Consider a Separate System

If any of the following become true, migrate procedural memory to a dedicated store:

1. **Volume** — Procedural rules accumulate significantly and cause retrieval noise when mixed with semantic facts
2. **Priority** — Behavioral rules need to be *always* included in context (not just when query-relevant), requiring a different retrieval strategy
3. **Structure** — Rules need richer structure than SPO (e.g., trigger conditions, priority ordering, context scoping)

The migration path is straightforward: extract facts where `subject = "assistant"` and procedural predicates into a new `procedural_rule` table.

## Implementation Plan

### Phase 1: Reuse Semantic Memory

- [ ] Extend LLM extraction prompt with procedural rule guidance
- [ ] Add procedural predicates to recommended predicate list
- [ ] Filter `subject = "assistant"` facts into a "Behavioral Guidelines" section in tool results

### Phase 2: Evaluate and Potentially Separate

- [ ] Monitor procedural fact volume vs semantic fact volume
- [ ] Assess whether procedural rules need always-on context injection (vs query-driven retrieval)
- [ ] If needed: dedicated `procedural_rule` table + migration

## What We Don't Do

- **No separate table in MVP**: Procedural rules reuse semantic memory infrastructure.
- **No rule engine**: Rules are natural language, interpreted by the consuming LLM, not executed programmatically.
- **No priority system**: All rules are equal. If ordering/priority becomes important, it's a Phase 2 concern.
- **No motor/skill memory**: We only model behavioral and communicative procedures, not physical skills.
