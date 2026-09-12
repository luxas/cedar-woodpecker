# Plan 8 — `cedar lint`: the policy simplifier as a Cedar CLI command

## Goal

The policy simplifier (branch 7) is a library function; users and editors need a command. The
Cedar CLI already integrates `cedar-policy-symcc` behind the experimental `analyze` feature as
`cedar symcc …`, with clap, shared schema/policy argument groups, miette reporting, a tokio
runtime and a cvc5 locator. This branch adds `cedar lint` next to it: human-readable findings by
default, `--json` for editors (the VS Code extension of the same branch runs it), and
`--deny-findings` as a CI mode.

## Design

```
cedar lint --schema <FILE> [--schema-format cedar|json] [--policies <FILE>] [--cvc5-path <P>] [--json] [--deny-findings]
```

- `LintArgs`: the shared `SchemaArgs` (flattened), `--policies` (Cedar syntax — the findings
  are byte offsets into it; stdin otherwise), `--cvc5-path` (env `CVC5`), `--json`,
  `--deny-findings`. `SchemaArgs::get_schema_with_source` returns the schema text alongside the
  parsed schema, which `collect_semantics` (branch 6) needs; `initialize_solver` is shared with
  `cedar symcc`.
- Body: read the policies, build the solver, compiler and `Evaluator`, collect the assumptions,
  run `simplify` on the runtime.
- Output. `--json`: the findings array exactly as the library serializes it, one line on
  stdout; errors as one plain line on stderr (an assumption error keeps its `the @semantics`
  prefix first, which the extension routes on); exit `Success` whatever the findings. Default:
  one miette report per finding — a `LintFinding` diagnostic of severity warning, the policy
  document as its source (named after the file or `<stdin>`), a label on the span, code
  `cedar::lint::<kind>` — then a summary line; errors through miette as `symcc` does.
- Exit codes: `Success`; `ValidationFailure` (3) with `--deny-findings` when any finding.
- Without the `analyze` feature the subcommand exists and prints the same "experimental, not
  built with `analyze`" stub as `symcc`. The experimental release archive is built with
  `analyze,tpe`, so it ships `cedar lint`.

## Files

- `cedar-policy-cli/src/command/lint.rs` (new), `src/command.rs`, `src/main.rs`,
  `src/command/symcc.rs` (solver helper visibility), `src/utils/schema.rs`, `tests/lint.rs`
  (new; `assert_cmd`, temp files, needs cvc5), `CHANGELOG.md`, `README.md`.

## Verification

`cargo test -p cedar-policy-cli --all-features` (the `lint` tests: a schema with `@semantics`
and an optional attribute, `principal has nick && principal.active` reporting a never-false
finding whose offset selects the guard, the human output containing the source line,
`--deny-findings` exiting 3, an unparsable policy file exiting 1, a rejected `@semantics`
printing `the @semantics` on stderr with `--json`), the stub without the feature, the existing
`symcc` tests untouched.

## History

Merged from the private plan "cedar lint, the policy simplifier as a Cedar CLI command". The
simplifier first shipped as a separate `cedar-symcc-simplify` binary with a hand-rolled argument
parser; it was replaced by this command and does not exist on this stack.
