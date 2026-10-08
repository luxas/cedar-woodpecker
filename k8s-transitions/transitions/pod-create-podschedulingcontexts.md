# create Pod → create PodSchedulingContext

**Paper:** Yang et al., CCS '25 §4 (implicit permissions of workload creation); severity **Medium** (classification.txt, `# medium` bucket).
Reference(s) formalized (from `assessment/trigger.txt`):
- `create pods --> create podschedulingcontexts`

**Status: NOT SYNTHESIZABLE (missing schema entity).** No `.json`/`.cedar` is
shipped, so `verify.sh` skips this mechanism.

## The implicit permission

Creating a Pod that references a `ResourceClaim` (Dynamic Resource Allocation)
causes the apiserver / DRA controller to create a `PodSchedulingContext` object
so a resource driver can coordinate scheduling. A principal holding `create pods`
thus causes a `create podschedulingcontexts` it was never granted.

## Why it exists (Kubernetes mechanism)

Under Dynamic Resource Allocation, a Pod with `spec.resourceClaims` cannot be
scheduled until its claims are allocated; the scheduler/DRA path drives this
through a per-Pod `PodSchedulingContext`, which is created on behalf of the Pod.
The referenced claims are the `core::pods.resourceClaimRefs` derived field
([`resourceclaim.PodClaims`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/staging/src/k8s.io/dynamic-resource-allocation/resourceclaim/pod.go#L35)).

## Why it is not synthesizable here

`cedar-woodpecker` typechecks a transition's `target.resource` against the
schema. This schema (`k8s.cedarschema`, generated from the API groups listed in
SCHEMA.md) contains **no** `podschedulingcontexts` entity and no
`resourceclaims` entity — the `resource.k8s.io` group was not among the imported
groups. A transition targeting a non-existent entity cannot be written or made
sound. The field `core::pods.resourceClaimRefs` exists in the Pod projection,
but there is no target entity for the implicit `create` to land on.

To synthesize this later, regenerate the base schema with the `resource.k8s.io`
API group included so that `resource::podschedulingcontexts` (or the current
`resourceslices`/`resourceclaims` shapes) become entities; then the transition
would be `source create/core::pods → target create/<the PSC entity>` with
`when: "resource.namespace == context.resource1.namespace"`.

## Modeling notes

- This is a schema-coverage gap, distinct from the deny-side case in HARD RULE 5:
  the mechanism IS a permit a principal gains (not a deny), so it would be
  synthesizable against a schema that modeled the entity.
- Severity Medium per the paper: a stray `PodSchedulingContext` is a scheduling
  side effect rather than a direct data read, but DRA coordination objects can
  influence where/whether workloads land.
