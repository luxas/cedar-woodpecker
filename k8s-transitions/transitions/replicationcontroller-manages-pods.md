# create/update/patch ReplicationController → create/update/patch/delete Pod

**Paper:** Yang et al., CCS '25 §3.3 (controller reconciliation); severity **Critical** (classification.txt).
Reference(s) formalized (from `assessment/trigger.txt`):
- `create replicationcontrollers --> create pods`
- `update replicationcontrollers --> create pods`
- `update replicationcontrollers --> patch pods`
- `update replicationcontrollers --> delete pods`
- `patch replicationcontrollers --> create pods`
- `patch replicationcontrollers --> patch pods`
- `patch replicationcontrollers --> delete pods`
- `delete replicationcontrollers --> delete pods`
- `deletecollection replicationcontrollers --> delete pods`

## The implicit permission

A principal holding create/update/patch (p) on `core::replicationcontrollers` in
a namespace can cause create/update/patch/delete (q) on `core::pods` in that
namespace, with no direct Pod verb. The ReplicationController is the legacy
predecessor of the ReplicaSet and reconciles identically — stamping Pods from its
pod template to match the desired replica count.

## Why it exists (Kubernetes mechanism)

The ReplicationController controller is literally a thin wrapper around the
ReplicaSet controller in `kube-controller-manager`, so the Pod create/delete
reconciliation is the same `manageReplicas` loop.
Source: [`ReplicationManager` (wrapper around `ReplicaSetController`)](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/controller/replication/replication_controller.go#L47) — `ReplicationManager` embeds `replicaset.ReplicaSetController`.
Source: [`ReplicaSetController.manageReplicas`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/controller/replicaset/replica_set.go#L653) — the shared create/delete-Pods loop.

## Transition function

Uses derived field(s): none (namespace-scoped reconciliation).

`replicationcontroller-create-creates-pod.json`:
```json
{ "transitions": [ {
  "name": "replicationcontroller-create-creates-pod",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"create\"", "resource": "core::replicationcontrollers" } ],
  "target":  { "action": "k8s::Action::\"create\"", "resource": "core::pods" },
  "when": "resource.namespace == context.resource1.namespace"
} ] }
```

## Example

Explicit policy (`replicationcontroller-create-creates-pod.cedar`):
```cedar
permit (
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is core::replicationcontrollers
);
```

## Synthesized implicit policy

```
// replicationcontroller-create-creates-pod: (principal: k8s::User, action: k8s::Action::"create", resource: core::replicationcontrollers) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"create", resource: core::pods) [sound]
// path: (context.action1 == k8s::Action::"create") && (resource.namespace == context.resource1.namespace)
@woodpecker("replicationcontroller-create-creates-pod: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is core::pods
);
```

## Modeling notes

- **Over-approximation:** namespace restriction, as with ReplicaSet.
- **Severity Critical:** identical template-controls-pod reasoning — the RC's pod
  template fixes `serviceAccountName`/secret refs, so RC write **chains into
  FAMILY A's `create Pod → get Secret`**.
