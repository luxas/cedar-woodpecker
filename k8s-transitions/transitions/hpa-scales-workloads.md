# create/update/patch HorizontalPodAutoscaler → patch Deployment/StatefulSet/ReplicaSet/ReplicationController

**Paper:** Yang et al., CCS '25 §3.3 (autoscaler controller); severity **High (DOS)** (classification.txt).
Reference(s) formalized (from `assessment/trigger.txt`):
- `create horizontalpodautoscalers --> patch deployments`
- `create horizontalpodautoscalers --> patch statefulsets`
- `create horizontalpodautoscalers --> patch replicasets`
- `create horizontalpodautoscalers --> patch replicationcontrollers`
- `update horizontalpodautoscalers --> patch deployments`
- `update horizontalpodautoscalers --> patch statefulsets`
- `update horizontalpodautoscalers --> patch replicasets`
- `update horizontalpodautoscalers --> patch replicationcontrollers`
- `patch horizontalpodautoscalers --> patch deployments`
- `patch horizontalpodautoscalers --> patch statefulsets`
- `patch horizontalpodautoscalers --> patch replicasets`
- `patch horizontalpodautoscalers --> patch replicationcontrollers`

## The implicit permission

A principal holding create/update/patch (p) on
`autoscaling::horizontalpodautoscalers` can cause patch (q) on the workload named
by the HPA's `scaleTargetRef` (`apps::deployments` / `apps::statefulsets` /
`apps::replicasets` / `core::replicationcontrollers`) in the same namespace, with
no direct verb on that workload. The HPA controller reconciles the autoscaler by
writing the target's `/scale` subresource — i.e. patching the target's
`spec.replicas` — which (via `scale-subresource-scales-pods.md`) then creates or
deletes Pods. So creating an HPA pointed at any scalable workload is a lever on
that workload's replica count.

## Why it exists (Kubernetes mechanism)

The HorizontalPodAutoscaler controller in `kube-controller-manager` computes a
desired replica count from metrics and writes it to the target's scale
subresource.
Source: [`HorizontalController` scale Update](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/controller/podautoscaler/horizontal.go#L1000) — `a.scaleNamespacer.Scales(hpa.Namespace).Update(ctx, targetGR, scale, ...)` writes `spec.replicas` on the `scaleTargetRef` workload.

## Transition function

Uses derived field(s): `autoscaling::horizontalpodautoscalers.scaleTargetKind`
and `.scaleTargetName` (see SCHEMA.md) — these project the HPA's
`Spec.ScaleTargetRef.{Kind,Name}` so the `when` can bind the implicit target to
the exact workload the HPA points at. Four `.json`/`.cedar` pairs ship, one per
target kind — `hpa-scales-deployment`, `hpa-scales-statefulset`,
`hpa-scales-replicaset`, `hpa-scales-replicationcontroller` — each differing only
in the `scaleTargetKind` string literal and the `target.resource`. All four were
validated `[sound]` (outputs below).

`hpa-scales-deployment.json`:
```json
{ "transitions": [ {
  "name": "hpa-scales-deployment",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"create\"", "resource": "autoscaling::horizontalpodautoscalers" } ],
  "target":  { "action": "k8s::Action::\"patch\"", "resource": "apps::deployments" },
  "when": "context.resource1.scaleTargetKind == \"Deployment\" && context.resource1.scaleTargetName == resource.name && resource.namespace == context.resource1.namespace"
} ] }
```

## Example

Explicit policy (`hpa-scales-deployment.cedar`):
```cedar
permit (
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is autoscaling::horizontalpodautoscalers
);
```

## Synthesized implicit policy

```
// hpa-scales-deployment: (principal: k8s::User, action: k8s::Action::"create", resource: autoscaling::horizontalpodautoscalers) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"patch", resource: apps::deployments) [sound]
// path: (context.action1 == k8s::Action::"create") && (context.resource1.scaleTargetKind == "Deployment") && (context.resource1.scaleTargetName == resource.name) && (resource.namespace == context.resource1.namespace)
@woodpecker("hpa-scales-deployment: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"patch",
  resource is apps::deployments
);
```

```
// hpa-scales-statefulset: (principal: k8s::User, action: k8s::Action::"create", resource: autoscaling::horizontalpodautoscalers) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"patch", resource: apps::statefulsets) [sound]
// path: (context.action1 == k8s::Action::"create") && (context.resource1.scaleTargetKind == "StatefulSet") && (context.resource1.scaleTargetName == resource.name) && (resource.namespace == context.resource1.namespace)
@woodpecker("hpa-scales-statefulset: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"patch",
  resource is apps::statefulsets
);
```

```
// hpa-scales-replicaset: (principal: k8s::User, action: k8s::Action::"create", resource: autoscaling::horizontalpodautoscalers) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"patch", resource: apps::replicasets) [sound]
// path: (context.action1 == k8s::Action::"create") && (context.resource1.scaleTargetKind == "ReplicaSet") && (context.resource1.scaleTargetName == resource.name) && (resource.namespace == context.resource1.namespace)
@woodpecker("hpa-scales-replicaset: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"patch",
  resource is apps::replicasets
);
```

```
// hpa-scales-replicationcontroller: (principal: k8s::User, action: k8s::Action::"create", resource: autoscaling::horizontalpodautoscalers) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"patch", resource: core::replicationcontrollers) [sound]
// path: (context.action1 == k8s::Action::"create") && (context.resource1.scaleTargetKind == "ReplicationController") && (context.resource1.scaleTargetName == resource.name) && (resource.namespace == context.resource1.namespace)
@woodpecker("hpa-scales-replicationcontroller: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"patch",
  resource is core::replicationcontrollers
);
```

- **Precision from derived fields:** unlike the other Family-B mechanisms (which
  reduce to the namespace restriction), this one keeps a tighter `when` because
  the HPA's `scaleTargetKind`/`scaleTargetName` name the exact workload — so the
  synthesized permit is `patch deployments` restricted to the HPA's named target
  in its namespace, not all deployments. Still an over-approximation only in that
  the authorizer sees the name match, not the live metric that would actually
  trigger a scale write.
- **DoS, not template control:** like `/scale`, the HPA can only move
  `spec.replicas` on its target — it cannot rewrite the pod template, so it does
  not grant the secret-mount escalation. Its power is availability/cost (scale to
  0 or scale up). Hence **High (DOS)**.
- The four target kinds are four separate `.json`/`.cedar` pairs sharing this md,
  each validated `[sound]` above. They swap the `scaleTargetKind` string literal
  (`"Deployment"`, `"StatefulSet"`, `"ReplicaSet"`, `"ReplicationController"`) and
  the `target.resource`.
