# delete/update/patch Node → delete Pod

**Paper:** Yang et al., CCS '25 §4.5.2 (the paper's worked static-analysis example:
"the `NodeController` invokes `deletePodHandler` when handling a node update
event … the explicit permission `update Node` leads to the implicit permission
`delete Pod`"); severity per `assessment/classification.txt`: **Critical** for
`update nodes --> delete pods` and `patch nodes --> delete pods` (the `# Critical`
bucket — tainting a Node evicts others' Pods), **High** (DoS) for
`delete nodes --> delete pods`.

Reference(s) formalized (from `assessment/trigger.txt`):
- `delete nodes --> delete pods`
- `update nodes --> delete pods`
- `patch nodes --> delete pods`

## The implicit permission

A principal who can `delete` a Node (or `update`/`patch` it to carry a
`NoExecute` taint, or to flip it `NotReady`) never receives `delete pods`, yet
the control plane deletes the Pods bound to that Node on their behalf. Deleting
the Node object orphans its Pods and the Pod GC controller force-deletes them;
tainting the Node makes the taint-eviction controller delete the Pods that do
not tolerate the taint. Either way the explicit Node write effects
`delete Pod` on every Pod scheduled there — a cross-namespace,
cluster-wide denial of service from a single Node permission.

## Why it exists (Kubernetes mechanism)

Pods carry `Spec.NodeName`; several controllers watch Nodes and act on the Pods
bound to them:

- **Pod GC — orphaned Pods.** When a Node object no longer exists, the Pod
  garbage collector force-deletes every Pod whose `Spec.NodeName` names the
  missing Node. Source:
  [`PodGCController.gcOrphaned`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/controller/podgc/gc_controller.go#L233)
  — it collects `existingNodeNames`, quarantines Pods whose `pod.Spec.NodeName`
  is not among them, and after the quarantine calls
  [`markFailedAndDeletePodWithCondition`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/controller/podgc/gc_controller.go#L264)
  on each.
- **Taint-eviction — `update`/`patch` Node.** Adding a `NoExecute` taint makes
  the taint-eviction controller delete Pods that do not tolerate it. The
  node-update handler is
  [`Controller.handleNodeUpdate`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/controller/tainteviction/taint_eviction.go#L533),
  and the work item it enqueues runs
  [`deletePodHandler`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/controller/tainteviction/taint_eviction.go#L107),
  which issues the Pod `Delete`. This is the exact `deletePodHandler` path the
  paper names in §4.5.2.

The implicit edge is the node-authorizer graph's `pod → node` relation
([`Graph.AddPod`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/plugin/pkg/auth/authorizer/node/graph.go#L361)
adds the `podVertex → nodeVertex` edge) read in the delete direction.

## Transition function

Uses derived field `core::pods.nodeName` (see `SCHEMA.md`; `Spec.NodeName`). The
Pod is deleted when it is bound to the Node being removed, so the condition
matches the Pod's `nodeName` to the Node's `name`.

`node-deletes-pods.json`:
```json
{ "transitions": [ {
  "name": "node-deletes-pods",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"delete\"", "resource": "core::nodes" } ],
  "target":  { "action": "k8s::Action::\"delete\"", "resource": "core::pods" },
  "when": "resource.nodeName == context.resource1.name"
} ] }
```

The `update`/`patch Node → delete Pod` rows are the same mechanism via the
taint-eviction controller rather than Pod GC; the transition shape is identical
with the source action changed to `k8s::Action::"update"`/`"patch"`.

## Example

Explicit policy (`node-deletes-pods.cedar`) — a node agent may delete its own Node object:
```cedar
permit(principal is k8s::User, action == k8s::Action::"delete", resource is core::nodes)
when { resource.name == "worker-1" };
```

## Synthesized implicit policy

```
// node-deletes-pods: (principal: k8s::User, action: k8s::Action::"delete", resource: core::nodes) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"delete", resource: core::pods) [sound]
// path: (context.action1 == k8s::Action::"delete") && (context.resource1.name == "worker-1") && (resource.nodeName == context.resource1.name)
@woodpecker("node-deletes-pods: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"delete",
  resource is core::pods
) when {
  resource.nodeName == "worker-1"
};
```

The synthesized policy keeps the `nodeName` restriction exactly (the derived
field relates the two requests without an intermediate to eliminate), so the
implicit `delete Pod` is scoped to Pods on `worker-1` — the Node the explicit
policy allowed deleting.

## Modeling notes

- **Cluster-wide blast radius.** With an unconstrained `delete nodes` grant the
  synthesized `delete Pod` is unconstrained too (all namespaces) — the DoS the
  paper's High bucket captures. `delete Namespace → delete Pod` is a sibling
  cascade (see `namespace-delete-cascades.md`); this one is keyed on scheduling,
  not namespace.
- **`update`/`patch` variants.** Modeled as the same mechanism (taint eviction).
  A tighter encoding would gate on the taint being `NoExecute`, which needs a
  derived `core::nodes.noExecuteTaints: Set<String>` field; left as future work.
- Severity: **Critical** for the `update`/`patch nodes → delete pods` rows
  (`classification.txt` `# Critical`) and **High** (DoS) for `delete nodes →
  delete pods`. The paper uses this
  edge as its canonical trace-based static-analysis finding.
