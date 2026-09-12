# Follow-ups for branch 4 — iferror

## PR description

Adds `iferror(e, d)`, a boolean extension function that evaluates to `e`, or to the fallback
`d` when `e` errors — Cedar's first way to say "treat an error as a value" inside a policy.
It exists for the DNF pipeline (branches 9–15), which moves every forbid into the permits it can
block and needs a forbid that *errors* (and is therefore ignored by the authorizer) to become
`false` rather than an error there. The function is available in policies, validated with the
signature `(Bool, Bool) -> Bool`, partially evaluated by TPE, and compiled symbolically as
`ite(is_none(e), d, e)`.

```cedar
permit(principal, action, resource) when {
  iferror(principal.age + 1 > 18, false)   // false, not an error, when `age + 1` overflows
};
```

```
iferror(true, <erroring>)  => true     (the fallback is never evaluated)
iferror(<erroring>, false) => false
iferror(1, true)           => type error (a non-boolean is not coalesced)
```

## What this branch contains

- Core: `extensions/iferror.rs` (registered like every extension; the coalescing itself is a
  special case in the concrete evaluator, the restricted evaluator and TPE, since an extension
  body only sees evaluated values), the validator signature, the PST/EST conversions.
- Symcc: `compile_iferror` in both compilers.

## Review findings

- **This is a language change.** A new extension function visible to every Cedar user
  (parser, validator, JSON/EST/PST forms, TPE, SymCC, the Lean spec). Upstream Cedar requires an
  RFC for a language change and a `cedar-policy/CHANGELOG.md` entry; neither exists. If it is
  meant to stay internal to the analysis tooling, gate it behind a feature (it is unconditional
  in `Extensions::all_available()`).
- Laziness of `d` is implemented by hand in three evaluators plus two symbolic compilers; the
  Lean model has the same shape. A shared helper on the `Value`/`PartialValue` side would
  reduce the copies.
- A theorem stating that the strict `call` body is unreachable with an erroring first argument
  would pin the invariant the comment states (cedar-spec side).

## Divergences from the private source

- None in content.

## Suggested follow-ups

- An RFC or a feature gate; a changelog entry; the invariant theorem.
