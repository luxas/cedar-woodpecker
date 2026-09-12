# Plan 1 — Setup: type-aware partial evaluation on by default for this branch stack

## Goal

The branches that follow add a symbolic evaluator, a DNF pipeline and a policy simplifier to
`cedar-policy-symcc`, all of which build on the type-aware partial evaluator (TPE) in
`cedar-policy-core` and `cedar-policy`. TPE sits behind the `tpe` feature, which is off by
default, so a plain `cargo build` or `cargo test` at the workspace root would compile none of the
TPE code paths those branches add to (`Residual::possible_bool_outcomes`, `to_typed_expr`, the
`iferror` arm of the TPE evaluator, `TypecheckedPolicySet::constant_fold`).

This branch switches the feature on by default in `cedar-policy-core` and `cedar-policy` for the
lifetime of the stack, so every branch is verified with TPE compiled and tested, and so that a
crate enabling `cedar-policy` without naming `tpe` (the workspace's default `cargo test`) still
exercises it. The change is marked temporary in both manifests and is to be reverted before a
release; the CI flow `cargo test --no-default-features` keeps checking the feature-less build.

## Design

- `cedar-policy-core/Cargo.toml`: `default = ["ipaddr", "decimal", "datetime", "tpe"]`.
- `cedar-policy/Cargo.toml`: `default = ["ipaddr", "decimal", "datetime", "tpe"]` (`tpe` there
  enables `cedar-policy-core/tpe`).
- Both carry a `# TEMPORARY:` comment stating the reason and that it is to be reverted.

Nothing else changes: the `experimental` feature of `cedar-policy-cli` already enables `tpe`,
and `cedar-policy-symcc` enables `cedar-policy/tpe` explicitly from the symbolic-evaluator
branch on, so removing the defaults again later leaves every consumer building.

## Verification

The workspace CI set with the defaults on: `cargo build`, `cargo build --features
"experimental,unstable-tooling"`, `cargo clippy --all-features`, `cargo doc --all-features
--no-deps`, `cargo test`, `cargo test --no-default-features`, `cargo test --features
"experimental,unstable-tooling"`, `cargo test -p cedar-policy-cli --all-features`.

## History

New for this stack; the private development enabled the feature in `cedar-policy-core` only,
with a note that it would be removed before merging.
