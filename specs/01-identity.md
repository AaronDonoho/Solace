# Spec 1: Identity and Revision Model

**Status:** accepted 2026-09-27 · **Implements:** PROPOSAL.md §4 · **Needed by:** Phase 1a

Every symbol has one stable identity and a sequence of revisions. Identity follows the operation performed, never textual similarity.

## 1. Symbols

Phase 1 symbol kinds and their identity prefixes:

| Kind | Prefix | Phase 1a (Gleam host) | Phase 1b (Solace) |
| --- | --- | --- | --- |
| Namespace | `ns` | module | `namespace` |
| Function | `fn` | top-level `fn` | `fn` |
| Type | `ty` | `type` | `type` |
| Constructor | `ct` | variant of a custom type | variant |
| Constant | `cn` | `const` | `const` |
| State machine | `sm` | (none) | `machine` |
| Test | `ts` | `pub fn *_test` | `test` |

Constructors are child symbols of their type: they have their own identity (references to a constructor survive reordering the variants), but they are edited only through their parent type's text.

Examples, contracts and effects are part of their function's content, not separate symbols. Decisions and work records are metadata (Spec 5), not code symbols.

## 2. Identity

- **Format:** `<prefix>:<16 lowercase hex>` (64 random bits), for example `fn:7f3a92c1e0b44d21`. Displays show the first 8 hex digits, extended as needed until unambiguous.
- **Assigned by the store**, never by an author. Never reused, including after retirement.
- **Nobody types an identity.** All commands take qualified names (§4). Identities appear in exports, history, traces and `--json` output.

## 3. Revisions

A revision is `sha256:` + the hex SHA-256 of the symbol's **normalized content**:

- **Included:** the canonical text (Spec 2) of the signature, effects, contracts, examples and body, with every reference to another symbol replaced by that symbol's identity. Parameter labels are included, because labels are API. **Local names are not** (D-6): each local binding is hashed by its binding position (like de Bruijn indices), so renaming a local has no semantic effect and creates no new revision.
- **Excluded:** the symbol's own name and namespace, local variable names, formatting (canonical anyway), and all prose metadata.

Consequences, each a test:

- R1. Renaming or moving a symbol leaves its revision unchanged.
- R2. Renaming a symbol leaves the revisions of all its dependents unchanged.
- R3. Editing only a description or rationale leaves the revision unchanged.
- R4. Two symbols with identical normalized content have identical revisions (duplicate detection, §6.3).
- R5. Reformatting submitted text leaves the revision unchanged.
- R6. Renaming a local variable leaves the revision unchanged; the new name is stored and shown in the projection.

Verification evidence attaches to `(identity, revision)`. When a revision changes, evidence for it becomes stale. So does evidence for any dependent that relied on something the change touched: its signature, effects or contracts. A body-only change leaves dependents' evidence fresh.

## 4. Names

- A **qualified name** is `namespace.path.name`, for example `pilot.pricing.quote`. Constructors are `namespace.Type.Variant`.
- Qualified names are unique among **live** symbols. A retired symbol's name is free to reuse, and the new symbol gets a new identity.
- **Resolution:** the input may be any suffix of a qualified name (`quote`, `pricing.quote`). Exactly one live match resolves; zero or several fail with error `E1001 unknown name` or `E1002 ambiguous name`, listing up to 10 candidates with their kinds and signatures.

## 5. Operations and their identity rules

Each operation is one store command (Spec 3) and records one history entry.

| Operation | Command | Identity result |
| --- | --- | --- |
| Create | `submit --new <ns>` | New identity |
| Edit | `submit <name>` | Same identity, new revision. The target named by the command decides identity, even if the text is completely rewritten |
| Rename | `rename <name> <new name>` | Same identity, same revision; history records the old name. Changes only the name (D-103) |
| Extract | `extract <symbols…> --to <module> [--to-project <folder>]` | Same identity and revision; history records the old module. Private helpers only they use come along, references and imports are rewritten (a symbol whose text changes gets a new revision), and a module or package cycle is refused (`E5002`, naming the boundary uses) (D-103). JavaScript FFI paths follow the module (`Rewrote FFI paths …`); into another project the files they name go along (`Moved file …`), refused with `E5007` when one can't; the reply names the suites that reach moved code, or says none does (D-124) |
| Merge | `merge <survivor> <retired> [--to-project <folder>]` | Survivor keeps its identity; the other is retired, linked `merged-into`, and all references are redirected, in one type-checked transaction. Refused when the two differ in kind or a type's constructors differ by name, arity or labels (`E5008`), when a project using the retired symbol can't import the survivor (`E5009`), or when a redirect closes a module or package cycle (`E5002`, naming the uses) (D-117) |
| Copy | `copy <name> <new name>` | New identity linked `copied-from` |
| Delete | `delete <name>…` | Each named symbol is retired in the order named, never erased. The reference check runs over the whole list first, so a symbol with live references from outside the list stops it and nothing is deleted (`E5001`, listing them) (D-102) |
| Import from export | `import <dir>` | See Spec 2 §5 |

A submitted text whose declared name differs from the target is an edit **and** a rename in one transaction.

## 6. Expression identifiers (answers §6.5 question 2)

Only *significant* expressions get identifiers: match arms, `if` branches, calls, holes, and contract/example clauses. They are numbered `#1, #2, …` in pre-order over the canonical syntax tree of each revision. Diagnostics and coverage use these numbers (`pricing.quote #17`).

Metadata that must outlive a revision (notes, and later coverage history) is anchored to an expression. On a new revision, the store re-maps each anchor with a tree diff between old and new syntax trees. An anchor that doesn't map cleanly is marked **orphaned** and listed in the change review. It is never silently dropped or moved.

## 7. Generics and derived code (answers §6.5 question 1)

Generic instantiations and derived implementations get **no identities of their own**. They are compile-time artifacts, never stored. Diagnostics, coverage and traces for derived code attribute to the deriving type's identity with a tag, for example `ty:7f3a…#Eq`.

## 8. Phase 1a mapping (Gleam host)

The store is SQLite. Imported Gleam modules are split into symbols per §1. Symbols and their references come from parsing with `glance` (the Gleam parser published as a Gleam package), resolved against each module's imports. Type information, where needed, comes from `gleam export package-interface`. Revisions hash the `gleam format` output of each symbol, with references replaced by identities. Gleam imports are regenerated from references and are not part of any symbol.

## Tests (acceptance)

R1 to R5 above, plus:

- I1. Delete with live references is refused and lists the referrers.
- I2. Rename, then create a new symbol with the old name, gives a new identity; the history of the renamed symbol still shows the old name.
- I3. Merge redirects every reference; afterwards the retired identity has zero live references.
- I4. Identities are never reused over 10,000 create/delete cycles.
- I5. An ambiguous suffix fails and lists every candidate.
