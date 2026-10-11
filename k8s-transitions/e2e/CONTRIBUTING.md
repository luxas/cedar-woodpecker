# Contributor brief — implicit-permission e2e tests

You are authoring end-to-end tests for implicit-permission transitions in
`/local/home/luxask/code/cedar-woodpecker-spec/cedar/k8s-transitions`. READ THIS
FULLY, then read `e2e/README.md`, `e2e/lib.sh`, and the five PROVEN reference
tests in `e2e/tests/` before writing anything.

## Environment (every shell MUST start with this)
```bash
export PATH="$HOME/.local/bin:$PATH"      # kind, kubectl
export WP_E2E_CONTEXT=kind-wp-e2e
```
The kind cluster `kind-wp-e2e` ALREADY EXISTS and is shared with other authors.
Node v1.37.0, authorizer `Node,RBAC`, image `busybox:1.36` preloaded on the node.
Do NOT run `setup.sh`/`teardown.sh`, do NOT `kind create/delete`, do NOT delete
the busybox image.

## What each test must prove (the contract)
For transition `<name>` (read `transitions/<name>.md` and `transitions/<name>.json`
for the exact `p`, `q`, and `when`), write `e2e/tests/<name>.sh`:
1. `source "$(dirname "$0")/../lib.sh"; test_start "<name>"`
2. Create a fresh identity (ServiceAccount) granted ONLY `p` via RBAC (helpers:
   `new_ns`, `mk_sa`, `grant_ns`, `grant_cluster`, `sa_subject`). Read-only grants
   needed purely to OBSERVE are fine but keep the write surface to exactly `p`.
3. `assert_can  <subj> <p-verb> <p-resource> [ns]`   (sanity: holds p)
   `assert_cannot <subj> <q-verb> <q-resource> [ns]` (THE GAP: lacks q)
4. Perform `p` as the identity with `as_subj <subj> ...` (impersonation).
5. Observe `q`'s effect as admin (plain `K ...`): a controller/kubelet/apiserver
   did `q`. Assert the effect with `assert_eq/contains/nonempty`, `wait_exists`,
   `wait_gone`, `wait_pod_running`.
6. End with `test_pass`.

Observation is done AS ADMIN on purpose: observing `q` usually needs `q`, which
the identity lacks. The identity performs `p` ONLY.

## Hard rules
- **Non-vacuous or honest SKIP.** The test must actually exercise p and observe
  q. If the mechanism cannot be reproduced on a vanilla single-node kind cluster
  (needs an aggregated apiserver, a metrics pipeline, a second node, a CRD/API
  group not installed, etc.), DO NOT fake it: print `RESULT: SKIP <name>
  <one-line reason>` and `exit 3`. A SKIP with a precise, true reason is a valid
  deliverable; a disguised pass is not.
- **The GAP must be real.** `assert_cannot` must pass BEFORE you act — if the
  identity can already do q, your RBAC grant is too broad; narrow it.
- **Namespaced only — do NOT mutate shared cluster state.** Use `new_ns` for a
  unique namespace per run. Cluster-scoped objects you create must be uniquely
  named (the helpers do this) and tracked for cleanup. **NEVER cordon, taint,
  drain, or delete a Node, and never touch another namespace** — tests run
  concurrently on the shared cluster and that would break other authors. If your
  transition inherently needs a node mutation, AUTHOR the test but add
  `exit 3` with `RESULT: SKIP <name> requires node mutation; parent runs it
  isolated` at the top so it does not run here (note it in your report).
- **Verify your own tests.** Run each with `bash e2e/tests/<name>.sh` (or
  `e2e/run.sh <name>.sh`) and iterate until it prints `RESULT: PASS` or a
  deliberate `RESULT: SKIP`. Report the actual observed output, not an assumption.
- **Clean up.** The harness auto-deletes tracked objects on exit; use the helpers
  so your objects are tracked. Keep per-test pods to a minimum (busybox, `sleep`).
- Commit nothing and touch no git state — the parent commits. Write ONLY files
  under `e2e/tests/`. Do not edit `lib.sh` (ask the parent if a helper is
  missing; you may add small local helper functions inside your own test file).

## Templates by shape (all five PASS live)
- kubelet mounts a referenced object: `pod-create-reads-secret.sh`
- controller stamps children from a template: `deployment-create-creates-pod.sh`
- apiserver/control-plane seeds an object: `namespace-create-seeds-serviceaccount.sh`
- subresource routes to the parent: `pod-eviction-deletes-pod.sh`
- RBAC bind escalation: `rolebinding-binds-role.sh`

## Report back
List each test file you wrote with its final status (PASS / SKIP+reason / FAIL+why),
the exact `auth can-i` gap you proved, and the effect you observed. Flag anything
you had to SKIP and why. Do not claim PASS you did not see printed.
