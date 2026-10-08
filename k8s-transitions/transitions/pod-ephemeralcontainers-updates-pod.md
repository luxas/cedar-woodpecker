# update/patch/get pods/ephemeralcontainers → update/patch/get Pod

**Paper:** Yang et al., CCS '25 §4 (subresource implicit permissions); severity **Critical** for the write rows, **Medium** for the read row (classification.txt: `update/patch pods/ephemeralcontainers --> …` sit in `# Critical`; `get pods/ephemeralcontainers --> get pods` sits in `# medium`).
Reference(s) formalized (from `assessment/trigger.txt`):
- `update pods/ephemeralcontainers --> update pods`
- `patch pods/ephemeralcontainers --> patch pods`
- `get pods/ephemeralcontainers --> get pods`

The three verbs are the one subresource↔parent mechanism; the `.json` below is
the `update` representative and the example policy permits all three.

## The implicit permission

Let *p* = `<verb> pods/ephemeralcontainers` and *q* = `<verb> pods`. The
`ephemeralcontainers` subresource is a view onto the same stored Pod object:
updating/patching it mutates the Pod's `spec.ephemeralContainers`, and getting it
reads the Pod. A principal holding only *p* therefore performs the corresponding
operation on the parent Pod without holding *q*. The edge is scoped to the same
Pod name and namespace.

## Why it exists (Kubernetes mechanism)

`EphemeralContainersREST` is a REST handler whose `Get`/`Update` operate on the
*parent Pod's* storage — it is a projection of the Pod, not a distinct object.
Updating it adds an ephemeral (debug) container to the running Pod; getting it
returns the Pod.

Source: [`EphemeralContainersREST`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/registry/core/pod/storage/storage.go#L386) — "implements the REST endpoint for adding EphemeralContainers"; its `Get`/`Update` read and write the parent Pod ([`Update` at L409](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/pkg/registry/core/pod/storage/storage.go#L409)).

## Transition function

Uses derived field(s): none (subresource→parent name+namespace match).

`pod-ephemeralcontainers-updates-pod.json`:
```json
{ "transitions": [ {
  "name": "pod-ephemeralcontainers-updates-pod",
  "principal": ["k8s::User"],
  "sources": [ { "action": "k8s::Action::\"update\"", "resource": "core::pods_ephemeralcontainers" } ],
  "target":  { "action": "k8s::Action::\"update\"", "resource": "core::pods" },
  "when": "context.resource1.name == resource.name && context.resource1.namespace == resource.namespace"
} ] }
```

## Example

Explicit policy (`pod-ephemeralcontainers-updates-pod.cedar`):
```cedar
// Alice may update/patch/get the ephemeralcontainers subresource of Pods.
permit (
  principal is k8s::User,
  action in [k8s::Action::"update", k8s::Action::"patch", k8s::Action::"get"],
  resource is core::pods_ephemeralcontainers
);
```

## Synthesized implicit policy

```
// pod-ephemeralcontainers-updates-pod: (principal: k8s::User, action: k8s::Action::"update", resource: core::pods_ephemeralcontainers) via policy0.cube0 -> (principal: k8s::User, action: k8s::Action::"update", resource: core::pods) [sound]
// path: (context.action1 == k8s::Action::"update") && (context.resource1.name == resource.name) && (context.resource1.namespace == resource.namespace)
@woodpecker("pod-ephemeralcontainers-updates-pod: policy0.cube0")
permit(
  principal is k8s::User,
  action == k8s::Action::"update",
  resource is core::pods
);
```

## Modeling notes

- The `.json` names the `update` verb; `patch` (same-verb mutation) and `get`
  (the read row) are the same subresource↔parent projection and are covered by
  the example policy. A separate `get→get` transition would be identical with
  the action swapped to `"get"`.
- **An ephemeral container can introduce new Secret references.** Adding a debug
  container via this subresource can set `env`/`envFrom`/`volumeMounts` that pull
  in Secrets, so this edge can chain into `pod-update-reads-secret`
  (`core::pods.secretRefs` grows). That chaining is why the write rows are
  Critical: ephemeral containers are a documented path to read mounted
  credentials and to run arbitrary images in an existing Pod's security context.
- Subresource→parent name+namespace match, the known-good shape from BRIEF.md.
