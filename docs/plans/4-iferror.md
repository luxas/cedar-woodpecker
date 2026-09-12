# Plan 4 — `iferror`: coalescing an error into a boolean

## Goal

The DNF pipeline (branches 9–15) rewrites a policy set into allow-only cubes by moving every
forbid's condition into the permits it can block. That is only sound if a forbid term that
*errors* — which makes the forbid ignored by the authorizer, so the permit still applies — does
not make the rewritten permit error too. Cedar has no operator that turns "this evaluated to an
error" into a boolean, so the pipeline needs one: `iferror(e, d)`, a function-style extension
function on booleans that evaluates to `e`'s value, or to `d`'s when `e` errors.

This branch adds the operator across the Rust stack: the extension, its special case in the
concrete evaluator, the restricted evaluator and the type-aware partial evaluator, its validator
signature, and its symbolic compilation. The Lean side (spec, typechecker, symbolic compilers,
TPE, proofs, DRT coverage) is the cedar-spec branch of the same name.

## Semantics

`iferror(e, d)`, both arguments and the result of type `Bool`:

| `e` evaluates to | result |
| --- | --- |
| `true` / `false` | that value; `d` is **not** evaluated |
| a non-boolean value | a type error (produced by `iferror` itself, not coalesced) |
| an error | `d`, coerced to a boolean: its value, or its own error |

So `iferror(e, false)` is exactly "`e` evaluates to `true`" as a Cedar boolean, and wherever `e`
is boolean-or-error (everything that validates) it never errors; `d` is a lazy fallback,
evaluated only when `e` errors, like every other second operand in Cedar. Under type-aware
partial evaluation an erroring `e` with a *partial* `d` yields the residual call
`iferror(<error>, d')`, not `d'`: returning `d'` would change the residual's type annotation and
break type preservation.

## Design

- **The extension** (`cedar-policy-core/src/extensions/iferror.rs`, registered with the others):
  one binary function-style function `iferror : (Bool, Bool) -> Bool`. An extension body only
  sees *evaluated* values and cannot observe its argument's error, so the coalescing is a
  special case in every evaluator *before* argument evaluation — the same shape the `unknown`
  function uses. The strict body (reached only when both arguments already evaluated) returns
  the first.
- **Evaluators**: `cedar-policy-core/src/evaluator.rs` (both the restricted and the full
  partial interpreters) and `src/tpe/evaluator.rs` evaluate the first argument themselves: a
  concrete value is coerced to a boolean (type error otherwise); an error makes them evaluate the
  fallback and coerce it; a partial first argument stays a residual call; a partial fallback
  behind an erroring first argument stays inside the call. Wrong arity falls through to the
  generic extension arm.
- **Validator** (`src/validator/extensions/iferror.rs`): the `(Bool, Bool) -> Bool` signature;
  typing stays strict (an unguarded optional access inside `iferror` is rejected like anywhere
  else).
- **Symbolic compilation** (`cedar-policy-symcc/src/symcc/compiler.rs`,
  `symccopt/compiler.rs`): symbolically an error is a `none` term, so `iferror(e, d)` is
  `ite(is_none(e), d, e)` over two `Option Bool` terms; the optimizing compiler unions the
  footprints. The lazy evaluation of `d` needs no encoding: terms are pure, and `d`'s own `none`
  only shows where it is selected.
- The PST/EST conversions know the function; the parser accepts it through the registration.

## Files

- `cedar-policy-core/src/extensions/iferror.rs` (new), `src/extensions.rs`, `src/evaluator.rs`,
  `src/tpe/evaluator.rs`, `src/pst/{ast_conversions,expr}.rs`,
  `src/validator/extensions/iferror.rs` (new), `src/validator/extensions.rs`,
  `src/validator/typecheck/test/extensions.rs`.
- `cedar-policy-symcc/src/symcc/compiler.rs`, `src/symccopt/compiler.rs`.

## Verification

Core: the rows of the table, laziness (`iferror(true, <erroring>)` is `true`), arity, nesting
under `!`, residual shapes under TPE, the validator's typing. Symcc: the compiled term shape,
literal folding, non-boolean arguments rejected. The Lean side proves the table and re-establishes
every soundness theorem with the new arm; the DRT generators emit `iferror` so every
type-directed differential target covers it (cedar-spec branch 4).

## History

Merged from the private plan "iferror — coalescing error → bool (Phase 3 Step 4, part 1)" and
its revisions; the allow/deny combination that motivates it is branch 15. Two things the
proofs forced are in the design above: the residual shape for a partial fallback, and the
opacity of `iferror` to the atom splitter (branch 10).
