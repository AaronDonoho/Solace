# Spec 4: Language Core (Phase 1 subset)

**Status:** proposed · **Implements:** PROPOSAL.md §8, limited to the §18.3 list · **Needed by:** Phase 1b

This spec defines only what Phase 1 builds. Anything else from §8 is listed under Deferred and is a compile error (`E0001 not yet supported`) until a later spec adds it. This file doubles as the draft of the language reference an agent reads at session start, so it must stay short.

## 1. Lexical

- `//` line comments; no block comments. `///` doc comments become the symbol's description (Spec 2 §1).
- Identifiers: `lowerCamel` for values and functions, `UpperCamel` for types, constructors, machines and states. Namespaces are dotted `lower` segments.
- Literals: `Int` (`1_000`), `Float` (`1.5`, and a literal must have digits on both sides of the point), `Text` (`"…"` with `\n \t \" \\ \u{…}` escapes), `True`, `False`, `()` for Unit.

## 2. Types

| Type | Meaning | BEAM representation |
| --- | --- | --- |
| `Int` | Arbitrary precision | integer |
| `Float` | IEEE 754 double; `NaN == NaN` is `False`; ordering puts `NaN` last | float |
| `Bool`, `Unit`, `Text` (UTF-8) | | atom, `ok`, binary |
| `List<T>`, `Map<K, V>` | Immutable | list, map |
| `Option<T>` = `Some(T) \| None` | Absence | `{some, X}` \| `none` |
| `Result<T, E>` = `Ok(T) \| Err(E)` | Failure | `{ok, X}` \| `{error, E}` |
| `fn(A, B) -> C` | Function values | fun |

User-defined types:

```
type Line { sku: Text, quantity: Int }                     // record
type Status = | Draft | Submitted | Cancelled(reason: Text)  // sum type
type OrderId = distinct Int                                // same representation, never mixes with Int
type Quantity = Int where 1 <= it and it <= 10             // refinement
```

- **Records** are constructed with every field labeled (`Line(sku: "A", quantity: 2)`), read with `.field`, and updated with `line with { quantity: 3 }`.
- **Distinct types** are converted explicitly: `OrderId(5)` and `id.value`.
- **Refinements:** a literal whose expected type is refined is checked at compile time (`E2010`). Any other value converts through `Quantity.from(x) -> Result<Quantity, Refinement>`. Arithmetic on a refined type yields its base type.
- **Generics** use angle brackets: `type Pair<A, B> { first: A, second: B }`. There are no interfaces and no type classes in Phase 1. `==` and `!=` are structural on every type except functions (`E2020`).

## 3. Functions

```
fn quote(lines: List<Line>, region: Region) -> Quote
    effects: none
    requires:
        lines != []
    ensures:
        result.total >= 0
    examples:
        quote(lines: [Line(sku: "A", quantity: 1)], region: Domestic) => Quote(…)
{
    let subtotal = sum(values: map(list: lines, with: lineTotal))
    …
}
```

- **Signatures are always written**: parameter types, return type and the `effects:` line. Types inside bodies are inferred (Hindley-Milner with let-generalization).
- **Argument labels are required** at call sites. The label may be omitted only when the argument is a variable with the same name (`quote(lines, region: r)`). The parameter name is the label.
- `requires:` and `ensures:` are boolean expressions, with `result` bound in `ensures`. In Phase 1 they are **checked at run time** on every call and fail with a contract violation that names the clause. Static proof is Phase 2.
- `examples:` lines are `call => expected value`. Each runs as a test (§7).
- **Expressions:** `let` (immutable), `if … { } else { }`, `match`, blocks where the last expression is the value, field access, record update, lambdas `fn(x) { … }`, and the pipe `|>`, which fills the first parameter.
- **`match` is exhaustive** (`E2030` lists the missing patterns). Patterns: constructors, literals, lists `[a, ..rest]`, `_`, bindings, and `if` guards.
- **Failure:** `fail("message")` is the only way to crash on purpose, and a function that can reach it declares the `Fail` effect. Failure otherwise goes through `Result`; there is no `try`/`catch`. A `?` suffix on a `Result` expression returns early on `Err`, and the error types must match exactly.

## 4. Effects as capabilities

- Phase 1 capabilities: `Clock` (`now() -> Int`, in Unix ms), `Random` (`int(min:, max:)`, `float()`), `Log` (`info(message:)`, `error(message:)`).
- A function uses a capability only through a parameter of that type. `effects:` lists the operations it and its callees use, for example `effects: Clock.read, Log.write`. Operation groups: `Clock.read`, `Random.read`, `Log.write`, plus `Fail`.
- The checker computes the exact effect set. A mismatch in either direction is an error (`E3001` undeclared, `E3002` declared but unused), with a fix.
- **Test fakes:** `Clock.fixed(at:)`, `Clock.stepping(start:, step:)`, `Random.seeded(seed:)`, `Log.capture()` together with `Log.lines(log:)`.
- **Entry points** are functions called from Erlang or the test runner. The runtime builds the real capabilities; nothing else can.

## 5. State machines

```
machine OrderStatus {
    states: Draft, Submitted, Approved, Shipped, Cancelled
    initial: Draft
    transitions:
        submit:  Draft -> Submitted
        approve: Submitted -> Approved
        ship:    Approved -> Shipped
        cancel:  Draft | Submitted -> Cancelled
    terminal: Shipped, Cancelled
}
```

- A machine defines a type whose values are its states, and one function per transition: `OrderStatus.submit(state:) -> Result<OrderStatus, InvalidTransition>`.
- The checker reports states unreachable from `initial` (`E4010`), transitions out of a terminal state (`E4011`), and non-terminal states with no outgoing transition (`W4012`).
- Phase 1 states carry no data, and transitions have no guards. Keep data in the enclosing record, and put guards in the calling function's `requires:`.

## 6. Typed holes

- `?` or `?name "question"` may stand for any expression. The program still type-checks. The checker reports each hole's expected type and the in-scope values that fit it (`W4001`).
- Reaching a hole at run time fails as **unfinished** (distinct from a wrong result). Each hole is recorded as verification debt, and a hole with a question raises a decision request (Spec 3 §5).

## 7. Tests

```
test "full checkout" {
    let clock = Clock.fixed(at: 1_000)
    …
    assert order.status == OrderStatus.Shipped
}
```

- `test` blocks are symbols. They may create fakes but no real capabilities. `assert <bool expr>` reports both sides of a failed comparison.
- `solace test` runs examples and tests. The outcomes are `pass`, `fail`, `contract violation` (naming the clause) and `unfinished` (naming the hole).

## 8. Namespaces

- Every symbol belongs to one namespace. Names resolve per Spec 1 §4 and render per Spec 2 §1. There are no import statements.
- Phase 1 has no `may depend on` / `may hold` boundaries (they arrive with the policy engine in Phase 2).

## 9. Compilation to Erlang (D-2)

- One Erlang module per namespace, named by the namespace (`'pilot.pricing'`). Functions become exported functions with parameters in declared order.
- Constructors become tagged tuples `{tag, Fields…}` (a bare atom when there are no fields). Records become `{TypeTag, Fields…}` in declared field order. Distinct types are erased. A refinement becomes a check function used by `from`.
- Contracts compile to checks at function entry and exit that raise `{solace_contract, Symbol, Clause, Values}`. Holes raise `{solace_unfinished, Symbol, Hole}`.
- Generated code is formatted Erlang with `-file` attributes mapping back to symbols, so BEAM stack traces name Solace symbols.

## Deferred (compile error until specified)

Units of measure, `Decimal`, sized integers, local mutation (`var`), interfaces, `derives`, versioned `stored`/`wire` types and migrations, namespace boundaries, actors and supervision, machine states with data and guarded transitions, `Database`/`Http`/`FileSystem`/`Config`/`Secrets`/`Spawn`/`Dom` capabilities, the JS backend, and foreign declarations. A consequence for the pilot: the store stays an immutable value passed through functions, as in the Gleam baseline.

## Tests (acceptance)

- L1. Golden files: each construct above has a source file with its expected diagnostics or its expected Erlang output.
- L2. The pilot, ported to Solace, passes the same behavior as the Gleam baseline's tests.
- L3. Every diagnostic code in this spec has at least one golden test that triggers it.
- L4. This spec, rewritten as the language reference, fits in 4,000 tokens.
