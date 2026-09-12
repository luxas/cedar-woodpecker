# Follow-ups for branch 8 — cedar-lint

## PR description

Adds `cedar lint` to the Cedar CLI (experimental `analyze` feature, next to `cedar symcc`): it
runs the policy simplifier on a schema and a policy document and reports every expression that
is statically never true, never false or always an error in every request environment — under
the schema's `@semantics` assumptions — as a warning pinned to its source span. `--json` gives
editors the findings with byte offsets (the VS Code extension uses it), `--deny-findings` makes
CI fail on any finding.

```
$ cedar lint --schema policies.cedarschema --policies policies.cedar
  ⚠ cedar::lint::never-true
   ╭─[policies.cedar:7:5]
 7 │     resource has request && resource.request.kind == "read"
   ·     ─────────┬─────────
   ·              ╰── `resource has request` is never true in every request environment
   ╰────
1 finding
$ cedar lint --schema policies.cedarschema --policies policies.cedar --json
[{"offset":123,"length":20,"kind":"never-true","message":"…"}]
```

## What this branch contains

- `cedar-policy-cli/src/command/lint.rs`, the `Commands::Lint` wiring and stub, the
  `get_schema_with_source` helper, `tests/lint.rs`, CLI changelog and README entries.

## Review findings

- The `--json` schema (`offset`, `length`, `kind`, `message`) is an interface with the VS Code
  extension but documented only there; `--help` or the CHANGELOG should state it, and a version
  field would let the extension detect drift.
- Two identical "experimental, not built with `analyze`" stubs (`symcc`, `lint`) could share a
  helper.
- The experimental release archive now ships `cedar lint`; worth a release-notes line.
- `tests/lint.rs` duplicates fixtures from `tests/symcc.rs`; a shared `tests/data/` would do.

## Divergences from the private source

- None in code.

## Suggested follow-ups

- Document the JSON contract; share the stub helper.
