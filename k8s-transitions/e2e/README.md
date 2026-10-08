# End-to-end tests for the implicit-permission transitions

Each transition in `../transitions/` claims an implicit permission `p -> q`:
holding only the explicit permission `p` causes Kubernetes to perform operation
`q` on the caller's behalf. The woodpecker transitions prove this *symbolically*
(cvc5, via `../verify.sh`). The scripts here prove the same thing
*operationally* — on a live [kind](https://kind.sigs.k8s.io) cluster.

## What a test proves

Every test follows one contract (see `lib.sh`):

1. **Grant only `p`.** A fresh ServiceAccount is bound, by RBAC, to the explicit
   permission `p` and nothing else.
2. **Confirm the gap.** `kubectl auth can-i q` for that identity returns `no` —
   it was never granted `q`.
3. **Exercise `p`.** Acting *as* that identity (impersonation, so RBAC is
   evaluated as the SA), the test performs `p` only.
4. **Observe `q`.** Kubernetes (a controller, the kubelet, or the apiserver)
   performs `q` on the identity's behalf, and the test observes the effect. The
   observation itself is done as the cluster admin, because observing `q`
   usually *needs* `q`, which the identity by construction does not have.

A test exits `0` for PASS, non-zero for FAIL, or prints `RESULT: SKIP <name>
<reason>` and exits `3` when the mechanism cannot be reproduced on a vanilla
single-node kind cluster (e.g. it needs an aggregated apiserver or a metrics
pipeline). SKIPs are honest gaps, never disguised passes.

## Running

```bash
./setup.sh          # create the kind cluster + preload the busybox image (idempotent)
./run.sh            # run every tests/*.sh, write results/summary.json
./run.sh pod-create-reads-secret.sh      # run a subset
./teardown.sh       # delete the cluster
```

Requires `kind`, `kubectl`, `docker` on `PATH`.

## The cluster

`kind-cluster.yaml` is a stock single-node kind cluster with two deviations,
both forced by the cgroup-v1 CI host and neither affecting authorization:
`failCgroupV1: false` (kubelet ≥1.36 refuses cgroup v1 otherwise) and the
`cgroupfs` cgroup driver for kubelet + containerd (the systemd driver's
`kubelet.slice` is not reliably delegated on this host). The authorizer is the
default `Node,RBAC`, which is what makes the node-authorizer edges
(`create Pod -> get Secret`, etc.) real.
