# Plan 6 — Semantic assumptions in the schema

## Goal

The policy simplifier (branch 7) pinpoints expressions that are statically determined with the
symbolic evaluator, but nothing tells it an authorizer's *conventions*: that `resource.request`
is populated only for write requests, say, so that a read policy guarded by
`resource has request.v1.type` is always false. The schema author knows such facts. This branch
lets them state them **in the schema**, as Cedar expressions the simplifier assumes true in
every request environment.

## Design

### The annotation

`@semantics("<Cedar boolean expression>")` on a namespace, an entity type or an action — the
declarations whose annotations `cedar_policy::SchemaFragment` exposes. The value is a Cedar
string literal (escape `\"` inside). One `@semantics` per declaration; several assumptions on
one declaration are conjoined with `&&`. Placement is only organisational: every assumption is
global, assumed true in every request environment.

```cedar
namespace k8s {
  @semantics("if resource is core::secrets && (action == k8s::Action::\"get\" || action == k8s::Action::\"list\") then !(resource has request) else true")
  entity User { groups: Set<String> };
}
```

Rules:

- an assumption must typecheck strictly, as a boolean, in **every** request environment of the
  schema — hence the `if <var> is <type> then … else true` guard, which is the author's job; the
  failing environment and annotation are reported otherwise;
- an assumption set that is unsatisfiable in some environment is an error (everything there
  would be vacuously determined; the simplifier checks this up front per environment);
- implications are written `if A then B else true`, not `!A || B`, for readability.

Common types are not enumerable through the fragment API, so a `@semantics` on a common type is
not collected yet. Two later iterations are kept open by the annotation's name and expression
form: `@semantics("file:…")` referring to a `.cedarsemantics` file, and *local invariants* of
common types over `this`, instantiated at each use of the type.

### `cedar-policy-symcc/src/semantics.rs`

`Assumption { origin, expression }` and `collect_semantics(schema_text, is_json, &schema) ->
Result<Vec<Assumption>, SemanticsError>`: the schema *text* is re-parsed as a `SchemaFragment`
(a parsed `Schema` drops annotations), namespaces, entity types and actions are walked, each
`@semantics` value is parsed as an `Expr`; `origin` names the declaration for messages. A
fragment or namespace that does not parse is an error, never a silent "no assumptions". Every
error message starts with `the @semantics`, the prefix editors route their warning on. The
evaluator exposes `typecheck_assumption(expr, env)` so a consumer can check every assumption in
every environment up front and name the failing one.

The simplifier (branch 7) passes the collected assumptions as per-call extra assumptions of
every evaluation.

## Files

- `cedar-policy-symcc/src/semantics.rs` (new; unit tests: each placement, an unparsable
  expression's message, no annotations), `src/lib.rs`.

## Verification

`cargo test -p cedar-policy-symcc --lib semantics`; the end-to-end cases (the Kubernetes shape,
an unguarded assumption naming the environment, a contradictory one, assumptions on all three
declaration kinds) are in `tests/simplify.rs` with branch 7.

## History

Merged from the private plan "semantic assumptions in the schema" and its two revisions; a
separate `.cedarsemantics` file format was drafted first and replaced by the annotation so that
the schema owns its semantics.
