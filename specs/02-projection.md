# Spec 2: Text Projection and Export Format

**Status:** accepted 2026-09-27 · **Implements:** PROPOSAL.md §4, §9.4 · **Needed by:** Phase 1a

Every revision has exactly one canonical text form. Agents read and write that text by symbol, one or many per call; the store handles identities, references and bookkeeping.

## 1. Canonical form

- One formatter, no options: Phase 1a uses `gleam format`, Phase 1b the Solace formatter. Projection is a pure function of the revision plus the names that the revision excludes: the symbol's own name, local names and the names of referenced symbols. The same inputs always give byte-identical text.
- **Names in text.** A reference renders as the shortest name that is unambiguous from the symbol's own namespace: a bare name within the namespace, otherwise the fully qualified name. Input may use any unambiguous suffix (Spec 1 §4); the formatter canonicalizes it.
  - One consequence: adding a symbol elsewhere can make a bare name ambiguous and change how an unedited symbol renders. That changes the projection, never the revision (references are stored as identities).
- **Prose is not in the text.** Descriptions and rationale are metadata, shown alongside the text by `inspect` (Spec 3). A doc comment in submitted text is split out into the description.

## 2. Single-symbol editing

`submit <name>` takes the complete new text for one symbol. In one transaction the store:

1. parses the text; the text must contain exactly one top-level definition (`E0101` otherwise);
2. resolves names to identities;
3. runs the fast check (Spec 3 §6) on the symbol itself;
4. if the symbol has errors, changes nothing and returns the diagnostics;
5. otherwise records the new revision, rechecks the symbol's dependents, and lists any it broke as **broken dependents** of the open change (Spec 3 §4).

There is no find-and-replace, no line numbers across symbols, and no partial-text edit. Line numbers exist only within a symbol's text.

## 3. Multi-symbol views

`view` renders several symbols as one document: a namespace, a list of names, the callers or callees of a symbol, or the symbols a change touched.

- **Order:** types, then state machines, then constants, then functions, then tests; alphabetical by qualified name within each group. Each symbol is preceded by a one-line marker comment: `// ── pilot.pricing.quote (fn) ──`.
- **Submitting a view:** the text is split at the marker comments.
  - A block whose marker names an existing symbol is an edit of that symbol.
  - A block without a marker is a new symbol, created in the view's namespace.
  - A symbol that was in the view but is missing from the submission is an **error** (`E0102`), never a deletion. Deletion is always explicit.
- All blocks apply atomically: if any block fails its check, nothing is recorded.

## 4. Export format

`export <dir>` writes the whole store as plain text, deterministically, in a Git-friendly layout:

```
<dir>/
  solace.toml                 # format version, toolchain version
  src/<namespace path>.sol    # one file per namespace (.gleam in Phase 1a)
  meta/<namespace path>.jsonl # metadata per symbol, one JSON object per line, sorted by identity
  retired.jsonl               # retired symbols: identity, last name, last revision, links
  history.jsonl               # all history entries in store order
```

In the source files, each symbol is one block, in the §3 order, preceded by an identity header:

```
// @id fn:7f3a92c1e0b44d21 @rev sha256:a83f…
pub fn quote(lines: List(Line), region: Region) -> Quote {
  …
}
```

Constructors take their identity header as a trailing comment on the variant line (`Draft // @id ct:…`). JSON is written with sorted keys and no insignificant whitespace.

## 5. Import

`import <dir>` rebuilds a store from an export. For each block:

| Case | Result |
| --- | --- |
| Header present, content unchanged | Same identity, same revision |
| Header present, content changed | Same identity, new revision (an edit made outside Solace) |
| No header | New symbol. If a retired or live symbol has the same qualified name, the import reports it as a *suggested match* and stops until the match is confirmed or rejected (`import --confirm <name>` / `--reject <name>`) |
| Same identity in two blocks | Error `E0103`; nothing imported |
| Header revision doesn't match its content | Treated as "content changed" |

## Tests (acceptance)

- P1. **Round trip:** `export → import → export` is byte-identical, for the whole pilot and after a random sequence of 500 operations.
- P2. Projection of a revision is byte-identical across calls and processes.
- P3. Submitting reformatted but otherwise identical text produces no new revision.
- P4. A view submission that omits a symbol fails with `E0102` and changes nothing.
- P5. A view submission with one bad block records none of its blocks.
- P6. An exported file edited by hand, with one symbol changed and one added without a header, imports as one new revision plus one new symbol.
