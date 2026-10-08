# update/patch <workload>/scale → create/delete Pod

**Paper:** Yang et al., CCS '25 §3.3 (scale subresource); severity **High (DOS)** (classification.txt).
Reference(s) formalized (from `assessment/trigger.txt`):
- `update deployments/scale --> create pods`
- `update deployments/scale --> delete pods`
- `patch deployments/scale --> create pods`
- `patch deployments/scale --> delete pods`
- `update replicasets/scale --> create pods`
- `update replicasets/scale --> delete pods`
- `patch replicasets/scale --> create pods`
- `patch replicasets/scale --> delete pods`
- `update statefulsets/scale --> create pods`
- `update statefulsets/scale --> delete pods`
- `patch statefulsets/scale --> create pods`
- `patch statefulsets/scale --> delete pods`
- `update replicationcontrollers/scale --> create pods`
- `update replicationcontrollers/scale --> delete pods`
- `patch replicationcontrollers/scale --> create pods`
- `patch replicationcontrollers/scale --> delete pods`

## The implicit permission

A principal holding update/patch (p) on the `/scale` subresource of a Deployment,
ReplicaSet, StatefulSet, or ReplicationController can cause create/delete (q) on
`core::pods` in that namespace, with no verb on the parent workload spec and no
Pod verb. Writing `/scale` sets `spec.replicas` on the parent, and the owning
controller then creates or deletes Pods to converge — so `/scale` write is a
narrow but real lever on the number of Pods (a denial-of-service / resource-abuse
primitive, and a way to force new Pods from an existing template).

## Why it exists (Kubernetes mechanism)

The apiserver's scale-subresource storage copies the submitted `scale.Spec.Replicas`
onto the parent workload's `spec.replicas`; the workload controller then
reconciles Pod count to the new value.
Source: [`scaleUpdatedObjectInfo` replicas write](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/registry/apps/deployment/storage/storage.go#L473) — `deployment.Spec.Replicas = scale.Spec.Replicas` (the equivalent exists for replicaset/statefulset/replicationcontroller scale storage).
Source: [`ReplicaSetController.manageReplicas`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/controller/replicaset/replica_set.go#L653) — creates/deletes Pods to the new replica count (StatefulSet/Deployment have analogous loops).

## Transition function

Uses derived field(s): none (namespace-scoped). One `.json`/`.cedar` pair per
scalable parent:

`deployment-scale-scales-pods.json`:
```json
{ "transitions": [ {
  "name": "deployment-scale-scales-pods",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"update\"", "resource": "apps::deployments_scale" } ],
  "target":  { "action": "k8s::Action::\"create\"", "resource": "core::pods" },
  "when": "resource.namespace == context.resource1.namespace"
} ] }
```

`replicaset-scale-scales-pods.json`, `statefulset-scale-scales-pods.json`, and
`replicationcontroller-scale-scales-pods.json` are identical except for the source
resource (`apps::replicasets_scale`, `apps::statefulsets_scale`,
`core::replicationcontrollers_scale`).

## Example

Explicit policy (`deployment-scale-scales-pods.cedar`):
```cedar
permit (
  principal is k8s::User,
  action == k8s::Action::"update",
  resource is apps::deployments_scale
);
```

## Synthesized implicit policy

```
// deployment-scale-scales-pods: (principal: k8s::User, action: k8s::Action::"update", resource: apps::deployments_scale) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"create", resource: core::pods) [sound]
// path: (context.action1 == k8s::Action::"update") && (resource.namespace == context.resource1.namespace)
@woodpecker("deployment-scale-scales-pods: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is core::pods
);
```

```
// replicaset-scale-scales-pods: (principal: k8s::User, action: k8s::Action::"update", resource: apps::replicasets_scale) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"create", resource: core::pods) [sound]
// path: (context.action1 == k8s::Action::"update") && (resource.namespace == context.resource1.namespace)
@woodpecker("replicaset-scale-scales-pods: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is core::pods
);
```

```
// statefulset-scale-scales-pods: (principal: k8s::User, action: k8s::Action::"update", resource: apps::statefulsets_scale) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"create", resource: core::pods) [sound]
// path: (context.action1 == k8s::Action::"update") && (resource.namespace == context.resource1.namespace)
@woodpecker("statefulset-scale-scales-pods: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is core::pods
);
```

```
// replicationcontroller-scale-scales-pods: (principal: k8s::User, action: k8s::Action::"update", resource: core::replicationcontrollers_scale) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"create", resource: core::pods) [sound]
// path: (context.action1 == k8s::Action::"update") && (resource.namespace == context.resource1.namespace)
@woodpecker("replicationcontroller-scale-scales-pods: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is core::pods
);
```

## Modeling notes

- **Narrower than full workload write:** `/scale` write only moves `spec.replicas`
  — it cannot change the pod template, so it does NOT, by itself, let the
  principal choose a new `serviceAccountName`/secret mount. Its escalation is to
  the *existence/count* of Pods matching the already-fixed template: scaling up
  spawns more Pods (resource exhaustion / scheduling abuse), scaling to 0 deletes
  all Pods (availability DoS). Hence **High (DOS)** rather than Critical — the
  secret-mount chain requires template control, which `/scale` lacks.
- **Over-approximation:** namespace restriction, as elsewhere.
