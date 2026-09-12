# Follow-ups for branch 7 — policy-simplifier

## PR description

Adds `cedar_policy_symcc::simplify`: given a schema, a policy document and the schema's
`@semantics` assumptions, it runs the symbolic evaluator over every policy in every request
environment and returns *findings* — the innermost expression that is never true, never false or
always an error wherever it is evaluated, and whole policies that fold to a constant — each with
its byte span in the document. A dead branch inside a live policy, a guard that contradicts an
authorizer convention, a condition that can only error: things the validator cannot see. It is
the library behind `cedar lint` (branch 8) and the VS Code warnings.

```rust
let (findings, _ev) = simplify(ev, &schema, policies_text, &assumptions).await?;
// [Finding { offset: 87, length: 9, kind: NeverTrue, message: "`a && !a` is never true …" }]
```

```cedar
permit(principal, action, resource) when { (a && b) || (a && !a) };
//                                                     ^^^^^^^^^  never true
```

## What this branch contains

- `cedar-policy-symcc/src/simplify.rs` (`simplify`, `Finding`, `FindingKind`, `SimplifyError`),
  `tests/simplify.rs`, `serde`/`serde_json` as dependencies.

## Review findings

- Findings are byte offsets into the *text* the caller passed; a JSON policy set has no source
  spans, so the API only works for Cedar syntax — say so in the signature's doc, or return
  `PolicyId` + `Loc`.
- The overlap rule ("innermost cause; a right operand judged only where the left can be true")
  is stated in prose; a test that pins the overlap behaviour per operator would keep it from
  drifting.
- `simplify` returns the `Evaluator` it was given so the caller can reuse the solver; a
  `&mut Evaluator` parameter would be the idiomatic shape.
- `SimplifyError`'s `the @semantics …` message prefix is an interface the VS Code extension
  matches (branch 8).
- Test coverage relies on cvc5 being installed; CI's symcc job has it, the others skip.

## Divergences from the private source

- `serde_json` is added here with `serde` although nothing in the library uses it before
  branch 8 moves the JSON printing into `cedar lint` (harmless to Cargo).
- Code comments cite the kept-guard rule as "plan 5" and this branch as "plan 7".
- The separate `cedar-symcc-simplify` binary of the private history is not part of this stack.

## Suggested follow-ups

- Spans by `PolicyId` + `Loc`; `&mut Evaluator`; a structured semantics error.
