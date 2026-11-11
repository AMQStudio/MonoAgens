# Middleware — Formats

Changes to a format bump its version.

## 1. Installed skill (SKILL.md)
Installed skills come from packages; they are static and never learned (D34). The common
SKILL.md format: YAML front matter, then the instructions in Markdown.

```markdown
---
name: shop-customer-service
description: Answer customer questions for a small shop politely and accurately.
version: 1
---
Steps …
```

Only `name` and `description` enter the prompt; the body is loaded when the PFC asks for it.
What the soul learns by itself is stored as `procedure` and `lesson` memories, not as SKILL.md.
