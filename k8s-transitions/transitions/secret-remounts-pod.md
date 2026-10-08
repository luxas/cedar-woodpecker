# patch/update `secrets` → patch/update the objects that mount them (`pods`, …)

**Paper:** Yang et al., CCS '25 (reverse data-flow of referenced objects); severity: `classification.txt` buckets the `patch secrets` rows as **medium** and `update secrets --> update pods` as **low**. Reported per row below.

Reference(s) formalized (from `assessment/trigger.txt`):

- `update secrets --> update pods` — low
- `patch secrets --> patch pods` — medium  *(runnable below)*
- `patch secrets --> patch deployments` — medium
- `patch secrets --> patch statefulsets` — medium
- `patch secrets --> patch daemonsets` — medium
- `patch secrets --> patch jobs` — medium
- `patch secrets --> patch cronjobs` — medium
- `patch secrets --> patch serviceaccounts` — medium
- `patch secrets --> patch ingresses` — medium
- `patch secrets --> patch persistentvolumes` — medium

## The implicit permission

Hold `p = patch secrets` (write a Secret's contents) and you effectively hold
`q = patch <object that mounts/references that Secret>`: by rewriting the Secret
you change the data that every mounting Pod sees at runtime, the TLS material an
Ingress serves, the credential a ServiceAccount hands out, the pull/credential a
PersistentVolume uses, etc. This is the **reverse** of the marquee
`create pods → get secrets` edge: there the Pod reaches into the Secret; here a
write to the Secret propagates *out* to every object that references it. The
principal never held write on the Pod, yet can mutate the Pod's effective
configuration.

## Why it exists (Kubernetes mechanism)

The kubelet does not snapshot Secret contents at Pod admission; it keeps a
**secret manager** that re-reads the referenced Secret and reconciles the live
value into the running Pod (env vars, mounted volume files, image-pull creds).
A `patch secrets` therefore changes what the already-admitted Pod runs with, no
Pod write required.

Source: [`secretManager.GetSecret`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/kubelet/secret/secret_manager.go#L84) — the kubelet resolves a Pod's referenced Secret through the manager at use time, not from an admission-time copy.

Source: [secret volume mounter re-reads via `getSecret`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/volume/secret/secret.go#L188) — `b.getSecret(b.pod.Namespace, b.source.SecretName)` fetches the current Secret when (re)populating the mounted volume, so a patched Secret surfaces in the Pod's files.

Source: [`getPullSecretsForPod` → `GetSecret`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/kubelet/kubelet_pods.go#L1132) — image-pull secrets are likewise resolved live from the manager for each pod, so editing the Secret changes what credentials the pod pulls with.

## Transition function

Uses derived field(s): `core::pods.secretRefs` (`Set<String>`, see SCHEMA.md —
the names a Pod references via imagePullSecrets, env/envFrom, volumes,
projected, computed by `VisitPodSecretNames`). The target request is the Pod;
the source is the Secret; the Pod fires only if its `secretRefs` contains the
patched Secret's name, in the same namespace (Secrets are namespace-local).

`secret-remounts-pod.json`:
```json
{ "transitions": [ {
  "name": "secret-remounts-pod",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"patch\"", "resource": "core::secrets" } ],
  "target":  { "action": "k8s::Action::\"patch\"", "resource": "core::pods" },
  "when": "resource.namespace == context.resource1.namespace && resource.secretRefs.contains(context.resource1.name)"
} ] }
```

## Example

Explicit policy (`secret-remounts-pod.cedar`):
```cedar
// A user granted patch on secrets in namespace "team-a".
permit (
  principal is k8s::User,
  action == k8s::Action::"patch",
  resource is core::secrets
)
when { resource.namespace.name == "team-a" };
```

## Synthesized implicit policy

`secret-remounts-pod`:
```
// secret-remounts-pod: (principal: k8s::User, action: k8s::Action::"patch", resource: core::secrets) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"patch", resource: core::pods) [sound]
// path: (context.action1 == k8s::Action::"patch") && (context.resource1.namespace.name == "team-a") && (resource.namespace == context.resource1.namespace) && resource.secretRefs.contains(context.resource1.name)
@woodpecker("secret-remounts-pod: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"patch",
  resource is core::pods
) when {
  resource.namespace.name == "team-a"
};
```

Reported `[sound]`.

## Modeling notes

- **Over-approximation (honest).** Woodpecker drops the
  `secretRefs.contains(...)` guard from the *synthesized* policy, widening it to
  the namespace restriction (`resource.namespace.name == "team-a"`). This is
  the expected derived-field widening (same as the `pod → secret` marquee): the
  transition condition is exact, but because the example policy constrains the
  source Secret only by namespace, the sound closed-form over-approximates to
  "any pod in the namespace". A per-Secret example would synthesize the tighter
  `secretRefs.contains(...)` form. This is a conservative (sound) widening, not
  an error.
- **The other mounters — same shape, different target entity, honest about
  derived fields:**
  - `serviceaccounts`: `core::serviceaccounts.secretRefs` **exists** (SCHEMA.md
    — Secrets holding the SA's legacy tokens), so `patch secrets → patch
    serviceaccounts` is expressible with the identical shape
    (`resource.secretRefs.contains(context.resource1.name)` on
    `core::serviceaccounts`, same namespace). The direction is slightly
    different in meaning (the SA *owns* the token Secret rather than *mounting*
    arbitrary data), but the derived field is present.
  - `deployments`, `statefulsets`, `daemonsets`, `jobs`, `cronjobs`: these are
    **workload controllers whose Pod template references the Secret**. The
    schema has **no** `secretRefs` field on these controller entities (only
    `core::pods` carries it). So a sound per-object transition cannot be written
    the same way; the honest formalization is that the escalation reaches them
    *transitively* through their managed Pods (the controller's template Secret
    == the Pods' `secretRefs`), which the `secret-remounts-pod` → Pod edge plus
    the workload→Pod edges (Family B) already compose. We do **not** fabricate a
    `secretRefs` field on controllers.
  - `ingresses`: an Ingress references a TLS Secret, but the schema has **no**
    `secretRefs` (or `tlsSecretRefs`) field on `io::k8s::networking::ingresses`,
    so there is no derived field to match on; a sound per-Secret transition is
    not expressible against this schema. Documented, not shipped.
  - `persistentvolumes`: a PV may reference Secrets (CSI node-publish / nodeStage
    secret refs), but `core::persistentvolumes` carries only `storageClassName`
    and `claimRef` — no Secret-ref derived field — so likewise not expressible
    here. Documented, not shipped.
- **`update secrets --> update pods`** is the same transition with the action
  `"patch"` swapped to `"update"` on both sides; sound by the identical
  argument, not shipped as a separate file.
- **Why medium (not low) for the patch rows:** editing a Secret silently changes
  the credentials/config of workloads that mount it — a path to privilege reuse
  or data tampering without ever touching the workload object. `update secrets
  --> update pods` is bucketed low in `classification.txt`; we report both as
  found rather than flattening.
