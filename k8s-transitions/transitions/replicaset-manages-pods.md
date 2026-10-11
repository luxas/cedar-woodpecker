# create/update/patch ReplicaSet → create/update/patch/delete Pod

**Paper:** Yang et al., CCS '25 §3.3 (controller reconciliation); severity **Critical** (classification.txt).
Reference(s) formalized (from `assessment/trigger.txt`):
- `create replicasets --> create pods`
- `update replicasets --> create pods`
- `update replicasets --> update pods`
- `update replicasets --> patch pods`
- `update replicasets --> delete pods`
- `patch replicasets --> create pods`
- `patch replicasets --> update pods`
- `patch replicasets --> delete pods`
- `delete replicasets --> delete pods`
- `deletecollection replicasets --> delete pods`

## The implicit permission

A principal holding create/update/patch (p) on `apps::replicasets` in a namespace
can cause create/update/patch/delete (q) on `core::pods` in that namespace,
without ever being granted a verb on Pods. The ReplicaSet controller reconciles
`spec.replicas` by stamping/deleting Pods from the ReplicaSet's pod template.

## Why it exists (Kubernetes mechanism)

The ReplicaSet controller in `kube-controller-manager` compares live Pods against
the ReplicaSet's desired replica count and creates or deletes Pods (from
`spec.template`) to converge.
Source: [`ReplicaSetController.manageReplicas`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/controller/replicaset/replica_set.go#L653) — the create/delete-Pods reconciliation loop (also shared by the ReplicationController, see `replicationcontroller-manages-pods.md`).

## Transition function

Uses derived field(s): none (namespace-scoped reconciliation).

`replicaset-create-creates-pod.json`:
```json
{ "transitions": [ {
  "name": "replicaset-create-creates-pod",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"create\"", "resource": "apps::replicasets" } ],
  "target":  { "action": "k8s::Action::\"create\"", "resource": "core::pods" },
  "when": "resource.namespace == context.resource1.namespace"
} ] }
```

## Example

Explicit policy (`replicaset-create-creates-pod.cedar`):
```cedar
permit (
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is apps::replicasets
);
```

## Synthesized implicit policy

```
// replicaset-create-creates-pod: (principal: k8s::User, action: k8s::Action::"create", resource: apps::replicasets) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"create", resource: core::pods) [sound]
// path: (context.action1 == k8s::Action::"create") && (resource.namespace == context.resource1.namespace)
@woodpecker("replicaset-create-creates-pod: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is core::pods
);
```

## Modeling notes

- **Over-approximation:** same namespace restriction as the Deployment case — the
  implicit permission is `create pods` for any pod in the ReplicaSet's namespace.
- **Severity Critical:** the ReplicaSet's pod template fixes the Pods'
  `serviceAccountName` / secret / configMap references, so ReplicaSet write
  **chains into FAMILY A's `create Pod → get Secret`** exactly as the Deployment
  case does. A Deployment create reaches here transitively
  (`deployment-manages-pods.md`), but a principal with direct ReplicaSet write
  (no Deployment) gets the same escalation one hop sooner.
