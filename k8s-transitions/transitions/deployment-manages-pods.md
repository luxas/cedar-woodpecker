# create/update/patch Deployment → create/update/patch/delete Pod (via ReplicaSet)

**Paper:** Yang et al., CCS '25 §3.3 (implicit permission via controller reconciliation); severity **Critical** (classification.txt).
Reference(s) formalized (from `assessment/trigger.txt`):
- `create deployments --> create replicasets`
- `update deployments --> create replicasets`
- `update deployments --> update replicasets`
- `update deployments --> patch replicasets`
- `delete deployments --> delete replicasets`
- `create deployments --> create pods`
- `update deployments --> create pods`
- `update deployments --> update pods`
- `update deployments --> delete pods`
- `patch deployments --> patch pods`
- `patch deployments --> create pods`
- `patch deployments --> delete pods`
- `delete deployments --> delete pods`
- `deletecollection deployments --> delete pods`

## The implicit permission

A principal holding create/update/patch (p) on `apps::deployments` in a namespace
can cause create/update/patch/delete (q) on `core::pods` (and on the intermediate
`apps::replicasets`) in that same namespace, even though it was never granted any
verb on Pods or ReplicaSets. The Deployment controller owns the reconciliation:
it stamps a ReplicaSet from the Deployment's pod template, and the ReplicaSet
controller then stamps Pods from that template. Deleting the Deployment cascades
(via owner references + garbage collection) to the ReplicaSets and their Pods.

## Why it exists (Kubernetes mechanism)

The `kube-controller-manager`'s Deployment controller reconciles each Deployment
by creating/scaling a ReplicaSet carrying the Deployment's `spec.template`; the
ReplicaSet controller in turn creates Pods from that template.
Source: [`DeploymentController.getNewReplicaSet`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/controller/deployment/sync.go#L146) — builds the new ReplicaSet and, at [sync.go#L232](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/controller/deployment/sync.go#L232), calls `ReplicaSets(d.Namespace).Create(...)`.
Source: [`ReplicaSetController.manageReplicas`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/controller/replicaset/replica_set.go#L653) — the ReplicaSet controller then creates/deletes Pods to match `spec.replicas`.

## Transition function

Uses derived field(s): none (namespace-scoped reconciliation).

`deployment-create-creates-replicaset.json`:
```json
{ "transitions": [ {
  "name": "deployment-create-creates-replicaset",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"create\"", "resource": "apps::deployments" } ],
  "target":  { "action": "k8s::Action::\"create\"", "resource": "apps::replicasets" },
  "when": "resource.namespace == context.resource1.namespace"
} ] }
```

`deployment-create-creates-pod.json`:
```json
{ "transitions": [ {
  "name": "deployment-create-creates-pod",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"create\"", "resource": "apps::deployments" } ],
  "target":  { "action": "k8s::Action::\"create\"", "resource": "core::pods" },
  "when": "resource.namespace == context.resource1.namespace"
} ] }
```

## Example

Explicit policy (`deployment-create-creates-pod.cedar`):
```cedar
permit (
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is apps::deployments
);
```

## Synthesized implicit policy

```
// deployment-create-creates-replicaset: (principal: k8s::User, action: k8s::Action::"create", resource: apps::deployments) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"create", resource: apps::replicasets) [sound]
// path: (context.action1 == k8s::Action::"create") && (resource.namespace == context.resource1.namespace)
@woodpecker("deployment-create-creates-replicaset: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is apps::replicasets
);
```

```
// deployment-create-creates-pod: (principal: k8s::User, action: k8s::Action::"create", resource: apps::deployments) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"create", resource: core::pods) [sound]
// path: (context.action1 == k8s::Action::"create") && (resource.namespace == context.resource1.namespace)
@woodpecker("deployment-create-creates-pod: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is core::pods
);
```

## Modeling notes

- **Over-approximation:** woodpecker proves the escalation only to the *namespace*
  restriction — the implicit `create pods` is granted for ANY pod in the
  Deployment's namespace, not just the pods owned by this Deployment. This is
  faithful to the authorizer's view: RBAC on Pods is namespace-granular and
  cannot name the owning Deployment, so a principal who can drive the Deployment
  controller to create pods in namespace `N` is, from the authorizer's
  standpoint, able to cause `create pods` in `N`.
- The two-hop chain (Deployment → ReplicaSet → Pod) is modeled as two sound
  single-hop transitions plus the direct Deployment → Pod closure; each is
  independently sound against the schema.
- **Severity Critical:** controlling the Deployment's pod template means
  controlling the Pods' `serviceAccountName`, `imagePullSecrets`, volume
  `secret`/`configMap` refs, etc. — this is exactly the lever that **chains into
  FAMILY A's `create Pod → get Secret`** (`core::pods.secretRefs` /
  `serviceAccountRef`): whoever controls the workload controls the pod template,
  so Deployment write subsumes the pod-level secret-mount escalation.
