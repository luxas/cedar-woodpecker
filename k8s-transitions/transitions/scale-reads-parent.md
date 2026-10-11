# get `<X>/scale` → get `<X>`  — NO TRIGGERED ROWS (nothing to formalize)

**Paper:** Yang et al., CCS '25. This file exists to record a scoping decision for Family F, not a transition.

## Finding

Family F was asked to cover any `get <X>/scale --> get <X>` style rows (a scale
subresource *read* that leaks the parent). Grepping the triggered reference set:

```
$ grep -E "/scale" assessment/trigger.txt | grep "get"
    (no output)
```

There are **no** `get <X>/scale --> get <X>` rows in `assessment/trigger.txt`.
So there is nothing to formalize for scale *reads*, and this mechanism ships no
`.json`/`.cedar` pair.

## Scale *writes* are out of scope for Family F

The `/scale` subresource rows that *do* trigger change replica counts and thus
create or delete Pods (e.g. `update deployments/scale`, `patch
replicasets/scale`, `update statefulsets/scale`). Those are **owned by Family B**
(the scale → create/delete Pods mechanism) and are deliberately **not**
duplicated here.

Had a `get <X>/scale --> get <X>` row existed, it would be the same
shared-store, object-identity mechanism documented in
`get-status-reads-parent.md` (a `/scale` GET returns the embedded `Scale`
object derived from the parent's spec/status via the same backing store), and
would use the identical `when` shape
(`resource.name == context.resource1.name && resource.namespace ==
context.resource1.namespace`). It is noted here only for completeness; it is not
synthesized because the row set does not require it.

## Status

`NOT APPLICABLE` — no triggered rows; no files shipped.
