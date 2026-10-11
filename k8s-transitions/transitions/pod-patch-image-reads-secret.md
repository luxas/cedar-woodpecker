# patch Pod (image swap) → get Secret

**Paper:** Yang et al., CCS '25 §4 (implicit permissions of workload mutation); also
DataDog **KubeHound** `POD_PATCH` ("Patch running pod" → *Container Administration
Command* → *Execution*). Severity **Critical** (classification.txt, `# Critical` bucket).
Reference(s) formalized (from `assessment/trigger.txt`):
- `patch pods --> get secrets`

Sibling verb/target not formalized here: `patch pods --> get configmaps` is the
same mechanism against a ConfigMap and is documented under
[`pod-update-reads-configmap.md`](pod-update-reads-configmap.md).

## The implicit permission

Let *p* = `patch pods` and *q* = `get secrets`. A principal holding *p* can
replace the `image` of a running Pod's container with an attacker-controlled
image. The kubelet kills the old container and starts the new one **in place** —
re-using the Pod's existing Secret volume mounts and its projected ServiceAccount
token. Attacker code now executes inside a container that already has those
Secret bytes on its filesystem, so it reads every Secret the Pod mounts without
the principal ever holding *q*. The edge is scoped to the Pod's namespace and the
Secrets it already references (`secretRefs`).

This is distinct from [`pod-update-reads-secret.md`](pod-update-reads-secret.md):
that edge models *adding a new* Secret reference by mutation, which
`ValidatePodUpdate` **rejects** (Pod spec immutability — see that file's honest
SKIP). The image-swap path here needs no new reference; it reads the Secrets the
Pod *already* mounts, and it is operationally reachable (proven live in e2e).

## Why it exists (Kubernetes mechanism)

`ValidatePodUpdate` forbids changing almost the entire Pod spec after creation,
but **`spec.containers[*].image`** (and `initContainers[*].image`) is on its small
allowlist of updatable fields. Replacing the image is therefore a legal `update`/
`patch`, and the kubelet reconciles the container spec by recreating the container
from the new image while leaving the Pod's volumes — including its Secret volumes
and projected SA token — mounted into the replacement. The attacker's image thus
runs with the Pod's full mounted-credential surface.

Source: [`updatablePodSpecFields`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/apis/core/validation/validation.go#L5862) — the allowlist naming `spec.containers[*].image` as a mutable field.
Source: [`ValidatePodUpdate`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/apis/core/validation/validation.go#L5875) — enforces that only those fields change on a Pod update/patch.
Source: [`VisitPodSecretNames`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/api/v1/pod/util.go#L113) — enumerates the Secret names the kubelet mounts (the `core::pods.secretRefs` derived field), unchanged by an image swap.
Source: [`Graph.AddPod`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/plugin/pkg/auth/authorizer/node/graph.go#L361) — the pod→secret edges whose mounted targets the swapped-in container can read.

## Transition function

Uses derived field(s): `core::pods.secretRefs` (see SCHEMA.md) — the Secrets the
Pod already mounts, which the swapped-in image inherits.

`pod-patch-image-reads-secret.json`:
```json
{ "transitions": [ {
  "name": "pod-patch-image-reads-secret",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"patch\"", "resource": "core::pods" } ],
  "target":  { "action": "k8s::Action::\"get\"", "resource": "core::secrets" },
  "when": "context.resource1.namespace == resource.namespace && context.resource1.secretRefs.contains(resource.name)"
} ] }
```

## Example

Explicit policy (`pod-patch-image-reads-secret.cedar`):
```cedar
// Alice may patch Pods in any namespace (e.g. to roll a container image).
permit (
  principal is k8s::User,
  action == k8s::Action::"patch",
  resource is core::pods
);
```

## Synthesized implicit policy

```
// pod-patch-image-reads-secret: (principal: k8s::User, action: k8s::Action::"patch", resource: core::pods) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"get", resource: core::secrets) [sound]
// path: (context.action1 == k8s::Action::"patch") && (context.resource1.namespace == resource.namespace) && context.resource1.secretRefs.contains(resource.name)
@woodpecker("pod-patch-image-reads-secret: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"get",
  resource is core::secrets
);
```

## Modeling notes

- **Bounded by `secretRefs`, not just namespace.** Unlike `pod-exec-reads-secret`
  (whose `core::pods_exec` subresource entity carries no `secretRefs`, so it drops
  to a namespace bound), the source resource here is `core::pods`, which carries
  the derived `secretRefs`. The implicit `get` is therefore confined to the
  Secrets the patched Pod already mounts — exactly what the swapped-in image can
  read — plus the namespace guard. A future Pod-admission policy that bounds
  `secretRefs` would bound this edge too.
- **Why `patch`, not a second `update` edge.** The twin
  `pod-update-reads-secret` carries the `update pods --> get secrets` row and
  documents the (blocked) add-a-ref path; this file carries the
  `patch pods --> get secrets` row and the (reachable) image-swap path. Both verbs
  can in principle do either thing; the catalogue files each row once, under the
  verb whose canonical exploit it names.
- **The SA token rides along.** The same image swap also inherits the Pod's
  projected ServiceAccount token, so `patch pods` is additionally a path to the
  Pod's SA credential (cf. `pod-create-mints-sa-token` /
  `serviceaccount-token-is-sa-credential`); the Secret read modeled here is the
  narrowest, most direct consequence.
- Severity Critical: direct read of mounted credentials by attacker-controlled
  code, obtained by mutating an existing Pod the attacker did not even author.
