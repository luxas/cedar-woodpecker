# create pods/exec → get Secret

**Paper:** Yang et al., CCS '25 §4 (subresource implicit permissions); severity **Critical** (classification.txt, `# Critical` bucket).
Reference(s) formalized (from `assessment/trigger.txt`):
- `create pods/exec --> get secrets`

Sibling subresource: `pods/attach` reaches the same running container the same
way (noted below); it is not a separate `trigger.txt` row.

## The implicit permission

Let *p* = `create pods/exec` and *q* = `get secrets`. Exec'ing into a container
drops the principal into a process that already has the Pod's mounted Secret
volumes and its projected ServiceAccount token on the filesystem (and in the
environment). The principal can `cat` those files, reading every Secret the Pod
mounts — without holding *q*. The edge is scoped to the Pod's namespace.

## Why it exists (Kubernetes mechanism)

`pods/exec` opens an interactive stream to a running container. The kubelet has
already mounted the Pod's Secret volumes and projected its SA token into the
container's filesystem (`/var/run/secrets/kubernetes.io/serviceaccount/token`
and any `volumes[].secret` mount), so an exec session can read them directly.
This is a runtime data path, not an apiserver read, which is why the implicit
`get Secret` is bounded only by the Pod's namespace rather than by a specific
`secretRefs` membership (the exec'd process sees whatever the Pod mounted).

Source: [`VisitPodSecretNames`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/api/v1/pod/util.go#L113) — defines which Secrets the kubelet mounts into the container the exec session enters.
Source: [`Graph.AddPod`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/plugin/pkg/auth/authorizer/node/graph.go#L361) — the pod→secret / pod→serviceAccount edges whose mounted targets the exec session can read.

## Transition function

Uses derived field(s): none (the exec subresource entity `core::pods_exec`
carries no `secretRefs`; the bound is the namespace).

`pod-exec-reads-secret.json`:
```json
{ "transitions": [ {
  "name": "pod-exec-reads-secret",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"create\"", "resource": "core::pods_exec" } ],
  "target":  { "action": "k8s::Action::\"get\"", "resource": "core::secrets" },
  "when": "context.resource1.namespace == resource.namespace"
} ] }
```

## Example

Explicit policy (`pod-exec-reads-secret.cedar`):
```cedar
// Alice may exec into Pods in any namespace.
permit (
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is core::pods_exec
);
```

## Synthesized implicit policy

```
// pod-exec-reads-secret: (principal: k8s::User, action: k8s::Action::"create", resource: core::pods_exec) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"get", resource: core::secrets) [sound]
// path: (context.action1 == k8s::Action::"create") && (context.resource1.namespace == resource.namespace)
@woodpecker("pod-exec-reads-secret: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"get",
  resource is core::secrets
);
```

## Modeling notes

- **Namespace-only over-approximation.** The `core::pods_exec` subresource entity
  exposes `name`/`namespace` but no `secretRefs` projection — the authorizer sees
  only that an exec on *some* Pod in the namespace is permitted, not which Secrets
  that Pod mounts. Woodpecker therefore drops to the sound namespace restriction:
  the implicit `get Secret` is granted for any Secret in the Pod's namespace. A
  tighter model would need the exec subresource to carry the parent Pod's
  `secretRefs`.
- `pods/attach` is the same story (it also joins the running container's
  filesystem), so a `create pods/attach --> get secrets` edge would be modeled
  identically; it is not currently a `trigger.txt` row.
- Severity Critical: exec is a direct, interactive read of mounted credentials
  including the Pod's SA token, which can then be replayed against the apiserver.
