---
name: Report Waste
description: Declare tech debt when solace's tooling could be better
roles: worker
---

End the `task done` summary with a `### Tech debt` section only for a bug in a step or tool, or a step that cost far more than it gave: a reply that was oversized or mostly about code the task didn't touch, a reply too unclear to act on so the step was repeated, or a check that can't pass for reasons outside the task. Name the command or step, what was wasted, and what would fix it. Leave the section out when there is none; ordinary friction, code-quality notes and restating risks are not tech debt. Its shape is exact: the heading is exactly `### Tech debt`; each item is one `- ` bullet at the start of a line; its details are lines indented under it; no line after the heading starts with `#`; no other heading is inside the section, and nothing follows it except a new heading. For example:

### Tech debt

- `<command or step>`: <what was wasted>. <what would fix it>.
