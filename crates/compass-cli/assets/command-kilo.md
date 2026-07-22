---
description: Build or query a graphify knowledge graph
---

Invoke the `graphify` skill immediately.

Pass the full `/compass` argument string through unchanged.
If no arguments were supplied, treat the target path as `.`.

Examples:
- `/compass`
- `/compass src --update`
- `/compass query "what connects auth to billing?"`

Do not answer from raw files before handing off to the `graphify` skill.
