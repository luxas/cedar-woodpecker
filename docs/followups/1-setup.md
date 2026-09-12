# Follow-ups for branch 1 — setup

## PR description

Turns the `tpe` feature (type-aware partial evaluation) on by default in `cedar-policy-core` and
`cedar-policy` while the symbolic-evaluator branch stack is developed, so that every `cargo
test` compiles and runs the partial evaluator that the following branches extend. No behaviour
changes for users who already enable `tpe`; users who do not now get the `tpe` API compiled in.
Marked temporary in both manifests.

```sh
cargo test -p cedar-policy-core           # now includes the tpe module's tests
cargo test --no-default-features          # still builds and tests the feature-less crates
```

## What this branch contains

- `cedar-policy-core/Cargo.toml`, `cedar-policy/Cargo.toml`: `tpe` added to `default`, with a
  `# TEMPORARY:` comment.
- `docs/plans/1-setup.md`, this file.

## Review findings

- Default features are additive across the workspace, so once any workspace member enables
  `tpe` the change is invisible to `cargo test` at the root; its effect is on per-crate
  builds (`cargo test -p cedar-policy-core`) and on downstream crates that do not name the
  feature. Revert before a release: a default `tpe` pulls in `partial-eval`-style code paths
  every consumer would otherwise opt into.
- The two comments should be removed together with the defaults; a single tracking issue is
  enough.

## Divergences from the private source

- `cedar-policy-core`: the source also had `tpe` in `default`, without the comment.
- `cedar-policy`: new here.

## Suggested follow-ups

- Revert both defaults when the stack has landed.
