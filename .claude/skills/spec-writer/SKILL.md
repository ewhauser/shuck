---
name: spec-writer
description: Write and update technical design specifications. Use this skill whenever the user wants to create a new spec, design doc, or technical specification, update an existing spec, or says things like "spec out X", "write a spec for Y", "create a design doc", "let's design Z", "add a spec", or "update the spec for W". Also trigger when the user is about to implement a significant feature and there's no spec for it yet — suggest writing one first.
---

# Spec Writer

Write and maintain technical design specifications that serve as the source of truth for architectural decisions.

Specs answer three questions: **what** are we building, **why** this approach over alternatives, and **how** do we verify it works. They're living documents — they start as proposals and evolve as implementation reveals new constraints.

## When to create vs. update

- **New spec**: The user wants to design something that doesn't have a spec yet, or is proposing a significant new feature/system.
- **Update spec**: The user is changing behavior that an existing spec covers, or implementation has diverged from what the spec describes.

Before writing anything, check the project's existing specs to understand the local conventions — numbering scheme, section structure, level of detail, and tone. Match what's already there.

## Process

### 1. Discover project conventions

Before drafting, read the project's existing specs to learn:

- **Where specs live** — look for a `specs/` directory, `docs/design/`, `rfcs/`, or similar
- **Naming convention** — numbered prefixes (`001-name.md`), date prefixes, or plain names
- **Section structure** — what sections exist and in what order
- **Style** — terse vs. prose-heavy, how much code is inline, whether there are diagrams
- **Status tracking** — do specs have a Status field? What values are used?

If the project has no existing specs, use the default structure below. If it does, match the existing style exactly — consistency across specs matters more than any "ideal" format.

### 2. Interview

Gather enough context to write a solid first draft. The goal is to understand the design space, not to exhaustively specify every detail — that comes through iteration.

Ask about:

- **What problem does this solve?** What's the motivation? Who benefits?
- **What's the proposed approach?** High-level design, key components, APIs
- **What alternatives were considered?** Why were they rejected?
- **What are the constraints?** Performance, compatibility, security, dependencies
- **How will we verify it works?** Tests, benchmarks, manual checks
- **What's the scope?** What's explicitly out of scope?

Don't ask all of these as a checklist — have a conversation. Some answers will be obvious from context, others will emerge as you discuss. If the user already has a clear picture, move quickly to the draft. If they're still figuring things out, the interview process helps them think through the design.

### 3. Draft the spec

Write the spec using the project's existing conventions. If there are no existing conventions, use this default structure:

```markdown
# NNN: Title

## Status

Proposed | Accepted | Implemented | Deprecated

## Summary

One paragraph: what this spec covers and why it exists.

## Motivation

Why is this needed? What problem does it solve? What's the current state?

## Design

The technical details. Use subsections as needed. Include:
- Architecture and component relationships
- API surfaces (with code examples)
- Data models and schemas
- Key algorithms or workflows
- Tables for structured comparisons
- ASCII diagrams for visual relationships

## Alternatives Considered

### Alternative A
Why it was rejected.

### Alternative B
Why it was rejected.

## Security Considerations

If applicable — threat model implications, trust boundaries, input validation.

## Verification

How to verify the spec is correctly implemented:
- Commands to run
- Tests to check
- Behaviors to observe
```

Guidelines for the draft:

- **Be concrete, not abstract.** Show code examples, API signatures, data structures. A spec that says "the system will handle errors gracefully" is useless. One that shows the error type and how callers handle it is useful.
- **Decisions over descriptions.** Every section should capture a decision and its rationale. If you're just describing how something works without explaining why it works that way, add the why.
- **Right-size the detail.** A 50-line spec for a simple feature is fine. A 500-line spec for a complex system is also fine. Match the complexity of the document to the complexity of the problem.
- **Include verification.** Every spec should end with concrete steps to verify the implementation matches the spec — specific commands, test names, observable behaviors.
- **Alternatives Considered is mandatory.** Even if the choice seems obvious, documenting what you didn't do (and why) prevents future engineers from relitigating the same decisions. If there truly were no alternatives, say so and explain why the approach was the only viable path.

### 4. Iterate

Share the draft and refine based on feedback. Common iteration patterns:

- User spots a missing edge case — add it to Design and possibly Verification
- User disagrees with an alternative's rejection — discuss and update
- Implementation reveals the design doesn't work — update the spec to match reality rather than leaving it stale
- Scope changes — update Summary and add/remove sections as needed

### 5. After the spec

Once the spec is accepted, offer to begin implementation if appropriate. The spec serves as the implementation plan — work through it section by section. As implementation progresses and the design evolves, keep the spec in sync. A spec that doesn't match the code is worse than no spec at all.

## Updating existing specs

When updating rather than creating:

1. Read the existing spec fully before making changes
2. Preserve the existing structure and style
3. Update the Status if the change warrants it (e.g., "Implemented" back to "Accepted" if redesigning)
4. Add to Alternatives Considered if a previously-rejected approach is now being adopted — explain what changed
5. Keep the Verification section current — if new behavior is added, add verification steps

## Numbering

When creating a new spec in a numbered series:

- Check existing specs for the highest number
- Use the next available number
- If multiple specs share a number prefix (e.g., `008-foo.md` and `008-bar.md`), that's OK — follow the project's convention
- Update any index or table of contents that references specs (like an AGENTS.md or README)
