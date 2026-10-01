---
version: 1
slug: "solace-cli-board-src-view-mjs"
primary_target: "solace-cli/board/src/view.mjs"
related_targets: []
---

# Contract view

Scope: the contract view (`/view`), the first slice of Spec 16 P4; the board follows later. Mode: Operate. Audience: a project owner, technical or not, who arrives equally from a task's review link or to explore. Desktop first; on a phone it must be readable.

Anti-goals: overclaiming (a derived fact must never look like a guarantee); hairball graphs (never draw every node at once).

## Direction contract

THESIS: the project as an engineering drawing sheet. It refuses the default arrangement of node-graph tools: dark canvas, rounded pastel cards and glowing bezier wires.

OWN-WORLD: drafting-film white sheet with a faint grid; black ink linework at two weights; one revision red, used only for review changes; kind colours as muted pencil tints in each box's corner tab. Boxes are square-cornered, with a name, a one-line summary and a short signature on the face. Connectors are orthogonal, with arrowheads like dimension lines: solid means observed or declared, dashed means derived. The title block sits at bottom right. Revision clouds go around changed nodes, removed nodes are drawn as phantom (long-dash) outlines, and A/B/C revision marks are listed in the title block.

STORY: the owner sees entry points and the effects they reach, opens what they need and judges each change. Nothing on the sheet is claimed beyond how it is known.

FIRST VIEWPORT: the sheet fills the window. The overview shows entry boxes on the left and effect boxes on the right, joined by dashed "reaches" connectors. The legend box is at top left (kinds, line meanings), the title block at bottom right. The search field sits in the sheet's top margin. The detail panel slides over the right side and is the only element with a soft shadow. In review, the decision panel replaces it and lists each revision mark in turn.

FORM: engineering drawing sheet; position 1 of the grounded list (Impeccable's pick, chosen by Aaron); seed key d60b1714.

FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, DESIGN.md, and every shipping raster carrying its provenance.

Behavior: expanding a node keeps it where it is and places its callees to the right, callers to the left and tests below; the nodes beyond slide over to make room, keeping their order; positions are remembered per viewer (localStorage, wrapped in try/catch). Keyboard focus on every node. Pan and zoom.

Later (not this slice): module group frames that collapse to one box, error-path connectors, runs drawn as movement along the connectors.
