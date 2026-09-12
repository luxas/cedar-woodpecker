# Follow-ups for branch 6 — semantic-assumptions

## PR description

Lets a schema state the conventions of its authorizer as `@semantics("<Cedar expression>")`
annotations on namespaces, entity types and actions. The policy simplifier (branch 7) assumes
them true in every request environment, so a policy that contradicts a convention — a read
policy guarded on an attribute only writes populate — is reported as statically false where it
happens. Assumptions must typecheck everywhere (guard them with `<var> is <type>`), and
implications read `if A then B else true`.

```cedar
@semantics("if action == Action::\"get\" then !(resource has request) else true")
entity Document = { request?: { kind: String } };
```

```
$ cedar lint --schema s.cedarschema --policies p.cedar     # branch 8
warning: `resource has request` is never true in every request environment
```

## What this branch contains

- `cedar-policy-symcc/src/semantics.rs`: `Assumption`, `collect_semantics`, `SemanticsError`;
  `Evaluator::typecheck_assumption`.

## Review findings

- `collect_semantics` needs the schema *text* because `cedar_policy::Schema` drops annotations;
  every caller threads a `String` alongside the `Schema` (`cedar lint` grows a
  `get_schema_with_source` for it). Keeping annotations on the parsed schema, or exposing them
  from `Schema`, would remove that.
- Common types cannot carry `@semantics` (the fragment API does not expose their annotations),
  so the planned local invariants have no place to live yet.
- The `the @semantics` message prefix is a string contract between the CLI and the VS Code
  extension (branch 8); a structured error would be more robust.

## Divergences from the private source

- None in content.

## Suggested follow-ups

- Annotations on the parsed `Schema`; local invariants over `this` for common types.
