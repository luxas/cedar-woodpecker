# create/update/patch/delete CustomResourceDefinition → manage its CustomResources

> **NOT SYNTHESIZABLE (deny-side / no target entity).** Registering a CRD makes a
> brand-new resource *type* exist; the custom resources it enables have **no
> entity in the generated schema**, so there is no `target` for a transition.
> `cedar-woodpecker` can only synthesize permits over entities the schema
> declares. No `.json`/`.cedar` ships and `verify.sh` skips it.

**Paper:** Yang et al., CCS '25; severity **High** (classification.txt).
Reference(s) formalized (from `assessment/trigger.txt`), 5 rows (the
status-subresource rows are a separate subresource→parent mechanism):

- `create customresourcedefinitions --> create customresources`
- `delete customresourcedefinitions --> delete customresources`
- `deletecollection customresourcedefinitions --> delete customresources`
- `update customresourcedefinitions --> update customresources`
- `patch customresourcedefinitions --> patch customresources`

(The sibling `update/patch customresourcedefinitions/status -->
update/patch customresourcedefinitions` rows are a subresource→parent *permit*
mechanism and belong to a different catalogue entry.)

## The implicit permission

A principal who may `create` a `CustomResourceDefinition` thereby causes a whole
new REST surface (`/apis/<group>/<version>/<plural>`) to come into existence —
the apiserver begins serving `create`/`get`/`update`/`patch`/`delete` on a
resource kind that did not exist before. Updating/patching the CRD reshapes that
surface (schema, versions, scope); deleting the CRD *removes* the type and causes
its stored custom resources to be garbage-collected. So CRD write implicitly
confers lifecycle control over a dynamically-defined resource family.

State plainly: p = `create|update|patch|delete customresourcedefinitions`;
q = existence/removal and shape of the `<plural>` custom-resource type and the
CRUD the apiserver will serve on it.

## Why it exists (Kubernetes mechanism)

apiextensions-apiserver watches CRDs and dynamically installs a per-CRD handler.
Once a CRD is Established, its handler serves full CRUD for the custom resource
kind; deleting the CRD tears the handler down and finalizes the stored objects.

Source: [`crdHandler.serveResource`](https://github.com/kubernetes/kubernetes/blob/788c4eca67a3ba878ead2385db1f4e274f9fd3a9/staging/src/k8s.io/apiextensions-apiserver/pkg/apiserver/customresource_handler.go#L368)
— switches on the request verb and wires `handlers.CreateResource` /
`UpdateResource` / `PatchResource` / `DeleteResource` / `DeleteCollection` for
the custom resource storage. The existence of this served surface is created by
the CRD itself.

## Why not a woodpecker transition

A transition's `target` must name an action over a concrete schema entity, and
woodpecker proves the synthesized `permit` sound against the schema's type
environment. Custom resources defeat that at the root:

1. **No target entity.** The custom resources enabled by a CRD are, by
   definition, *not* known at schema-generation time — the generated
   `k8s.cedarschema` has no `::<plural>` entity for them. A transition
   `target` referencing a non-existent entity cannot typecheck, so there is
   nothing to synthesize or prove sound.
2. **Type-existence, not a permit.** The forward effect is "a new resource *type*
   now exists / its surface changes," which is a schema-level change, not an
   additional `(action, resource)` permit over the fixed schema woodpecker
   reasons about.
3. **The `delete CRD --> delete customresources` rows** are likewise
   deletion/garbage-collection of a type's instances, again over an entity the
   schema does not model — not an additive permit.

No transition JSON/Cedar is emitted; the entry is marked not synthesizable
because the (dynamic) custom resources have no schema entity to target.

## Modeling notes

- Severity **High**: CRD write lets a principal mint arbitrary new API types
  (and the controllers watching them), or delete a type and cascade-delete every
  instance — broad, cluster-scoped lifecycle power.
- The limitation is fundamental to this catalogue's static schema: woodpecker
  escalates over a *fixed* entity set, whereas CRDs *add entities*. Capturing it
  would require a schema that models `k8s::Resource` as a catch-all AND a notion
  of "principal defines the type," which the current generated schema does not
  express — hence left out deliberately rather than approximated.
