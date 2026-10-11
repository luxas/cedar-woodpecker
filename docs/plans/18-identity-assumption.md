# Plan 18 — Identity assumption via principal lowering

> **Phase 1 deliverable: this plan + a feasibility review.** No code in this phase.
> Branch `identity-assumption`, stacked on `17-k8s-transitions`.

## Goal

Model **identity assumption** — a principal that, by exercising one permission,
comes to act *as another identity* and thereby gains that identity's
permissions. The Kubernetes catalogue (`KUBEHOUND-GAP.md`) has three instances
this feature must cover:

- **#1 `impersonate`** — `P impersonate serviceaccounts/<S>` ⟹ P can do anything
  `S` can (RFC k8s user-impersonation; the highest-impact escalation).
- **#2 read-token-Secret → assume SA** — `P get/list secrets` where the Secret is
  a `kubernetes.io/service-account-token` of SA `S` ⟹ P can do anything `S` can.
- **the create-pods chain** — `P create pods` with `serviceAccountName = S` ⟹ the
  pod runs as `S`, so P (via the pod) can do anything `S` can. This is the
  user's worked example `create pods -> impersonate serviceaccount`.

All three share one shape KubeHound draws as a `→ Identity` edge: *the acquired
capability is every cube the assumed identity holds, parametric over
`(action, resource)`* — not a single target cube. Today's woodpecker cannot
express it, and plan 16 says so outright under *Deliberately not done*:
"transitions that change the principal (impersonation) are outside the
transition signature". This plan removes that restriction with the **minimum**
change to the core and a preprocessing **lowering** that desugars the
`→ Identity` edge into ordinary single-target transitions the existing
`escalate` already handles soundly.

## Background — today's model (grounded in the code)

- A transition is `(pt, [(at_i, rt_i)], at', rt', T)` (`src/transition.rs`):
  one **principal type `pt` shared by every source and the target**
  (`SourceEnv::env(principal)` builds every `RequestEnv` with the same
  principal), source envs `(at_i, rt_i)`, target env `(at', rt')`, and a Cedar
  condition `T`.
- `escalate` (`src/escalate.rs`) iterates transitions × `pt`. For each source env
  it collects the policy set's **source cubes** there (`src/cubes.rs`); if any
  source env has none it skips. It builds an **extended schema**
  (`src/schema.rs`) that gives the target action's context the attributes
  `action{i}`, `resource{i}`, `context{i}` — the *i*-th intermediate request —
  then combines: pin `context.action{i} == at_i`, disjoin the renamed source-cube
  conditions per source, conjoin over sources, conjoin `T`. DNF-split + symbolic
  fold ⇒ **paths**; `eliminate` rewrites away every mention of the intermediate
  request (any attr in `added`), leaving the implied policy's `when` over the
  original schema. Each path is checked **sound** by the solver (path ⟹ policy).
- `T` may mention `principal`, `action`, `resource`, `context` (target) and
  `context.action{i}` / `context.resource{i}` / `context.context{i}`. **There is
  no `context.principal{i}`** — because all principals are `pt`, there is nothing
  to reference.

The single-principal assumption is the *only* thing in the way. Everything
else — the DNF, the symbolic soundness check, the elimination — is principal-
agnostic and stays untouched.

## The feature, in two layers

### Layer A — per-source principals + `context.principal{i}` (the one core change)

Give each request environment its own principal, exactly mirroring how each
already has its own action/resource/context:

1. **Signature.** `SourceEnv` gains an optional `principal: Option<EntityTypeName>`.
   The transition keeps a **required target principal** (the existing
   `principal` field, re-read as the *target* principal type set; still
   defaultable to "every applicable type" when omitted).
2. **Extended schema** (`extend_schema`). Add a `principal{i}` context attribute
   to the target action's context, typed as source *i*'s principal entity type
   (an entity-typed context attribute — Cedar supports these). `added_attrs()`
   gains `principal{i}`, so `eliminate` drops it like the others.
3. **`escalate`.** Build each source's `RequestEnv` with *its own* principal
   (set, or defaulted to the target principal); collect that env's cubes (for a
   source whose principal is `k8s::ServiceAccount`, these are the SA's cubes,
   independent of the target principal). In the combined condition, for each
   source *i* also pin `context.principal{i}` to the source env's principal, and:
   - **default-identity mode** (source principal unset): add
     `context.principal{i} == principal` — the identity flows through unchanged.
     This **reproduces today's semantics exactly** (backward compatible: the 63
     existing k8s transitions behave identically).
   - **explicit mode** (source principal set): add **no** default binding; `T`
     states the relation itself via `context.principal{i}`.
4. **`eliminate`.** No change beyond `principal{i} ∈ added`: the binding
   equalities (`context.principal{i} == principal`, or `context.principal{i} == …`)
   feed the existing equivalence-class rewrite, so the assumed identity's cube
   condition is re-expressed over the surviving variables and the intermediate
   principal is eliminated — the same mechanism as resource elimination.

The escalate **soundness machinery does not change** — the DNF split, the
symbolic per-path `path ⟹ policy` check and `eliminate` are principal-agnostic.
But principal is **not** a drop-in analog of resource (correction from the
feasibility review, Finding 1 — a blocker): source-cube conditions reference the
bare `principal` variable, so `rename_vars` **must gain a principal argument** and
remap `Var::Principal → context.principal{i}` per source. Otherwise a source whose
principal is the *assumed* identity (`k8s::ServiceAccount`) leaves its cube's
`principal.*` guards bound to the *acting* principal, silently mis-stating the
acquired capability. (Resource escaped this only because a cube never references a
*different* source's bare `resource`.) `Transition::principal_types()` must
likewise intersect candidates over **default-identity sources only** — a
set-principal source is validated against its own fixed type, not folded into the
acting-principal candidate set.

> **Design question (for the review): all-or-nothing vs per-source.** The task
> proposed two modes — *all* source principals unset (default identity) **or**
> *all* set. The lowering below needs a **mix**: the acting source (`create pods`)
> shares the target principal (unset/default), while the intermediate source
> (`ServiceAccount <action> <resource>`) is set. Two ways to honor this:
> - **R1 (recommended): per-source optional principal.** Each source is
>   independently unset (⇒ `== principal`) or set. Strictly generalizes today;
>   the mix is natural. This plan assumes R1.
> - **R2 (literal all-or-nothing):** keep the two modes; in the set-mode use a
>   reserved sentinel principal (e.g. `principal`/`__self`) for the acting source,
>   resolved to the iterated target type, and write `context.principal{i} ==
>   principal` by hand. Preserves the task's spec but needs a sentinel.
> R1 is simpler and subsumes R2; recommending R1.

### Layer B — `impersonation-transitions.json` + lowering (pure preprocessing)

A concise impersonation edge, desugared into many ordinary transitions **before**
`escalate` runs — no change to escalate semantics.

**Input format** (`impersonation-transitions.json`):
```jsonc
{ "impersonations": [ {
  "name": "create-pod-assumes-sa",
  "principal": ["k8s::User"],                 // the ACTING principal type(s) (target)
  "sources": [ { "action": "k8s::Action::\"create\"", "resource": "core::pods" } ],
  "assume": "k8s::ServiceAccount",             // target is just a PRINCIPAL TYPE
  "when": "context.principalX.name == context.resource1.serviceAccountRef && context.principalX.namespace == context.resource1.namespace"
} ] }
```
`assume` is the identity the acting principal comes to hold; `when` is the
**binding** that says *which* entity of that type (here: the SA the pod runs as).

**Lowering** (`<original principal> create pods` × `serviceaccount <action>
<resource>` → `<original principal> <action> <resource>`): for every request
environment `(a, r)` the `assume` principal type can act in **and for which the
input policy set actually has a source cube** (pruning — see Risks), emit one
ordinary Layer-A transition:
```
sources: [ <the impersonation sources, acting principal = target>,
           { principal: <assume>, action: a, resource: r } ]   // the assumed id's permission
target:  { action: a, resource: r }        // acting principal (= target principal)
when:    <impersonation.when>               // the binding, APPENDED to
         && context.action{k} == a && context.resource{k} == resource
         && context.action' (target) == a
```
i.e. the assumed-identity source's env is tied to the **target** env (same
action/resource), and the impersonation `when` ties that source's principal to
the identity the acting principal assumes. The lowered transition is then fed to
`escalate` unchanged; a path exists **iff** the acting principal holds the
source permission *and* the assumed identity actually holds `(a, r)` *and* the
binding holds — which is exactly sound identity-assumption semantics. The
impersonation `when` is literally appended to each lowered transition's `when`,
as the task specifies.

This is why the design is sound without new trust: the assumed identity's
permission is a **real source cube** (source 2) subject to the existing per-path
solver check, not an assumption.

## Worked examples (all three gaps)

| Gap | acting source | assume | binding `when` | note |
|---|---|---|---|---|
| **#1 impersonate** | `P impersonate io::k8s::authentication::serviceaccounts` | `k8s::ServiceAccount` | `context.principalX.name == resource.name && context.principalX.namespace.name == resource.namespace` | the impersonate resource is a *distinct* entity (`io::k8s::authentication::serviceaccounts`, `namespace: String`) from the SA principal, so bind by name and project `k8s::Namespace.name` (Findings 2 & 4) — **not** `== resource`, which is a cross-type comparison |
| **#2 token-Secret** | `P get core::secrets` | `k8s::ServiceAccount` | `context.resource1.type == "kubernetes.io/service-account-token" && context.principalX.name == context.resource1.serviceAccountName && context.principalX.namespace == context.resource1.namespace` | needs the gap-#2 `secrets.serviceAccountName` derived field |
| **create-pods chain** | `P create core::pods` | `k8s::ServiceAccount` | `context.principalX.name == context.resource1.serviceAccountRef && context.principalX.namespace == context.resource1.namespace` | uses existing `pods.serviceAccountRef` |

After lowering + escalate, each yields the family `P <a> <r> ⟹ P <a> <r>` wait —
`P <source> ⟹ P <a> <r>` for every `(a,r)` the SA holds, e.g. `P create pods ⟹ P
get secrets` when the pod's SA can `get secrets`. That is the KubeHound `→
Identity` edge, expressed as sound single-hop transitions.

## Feasibility (summary — the review audits this)

**Strongly feasible.** The pivotal schema facts are verified:
- `k8s::ServiceAccount` (and `User`, `Node`) **is a principal type for every
  action** (`k8s.cedarschema` action `principal:` lists), so
  `ServiceAccount <action> <resource>` environments — the intermediate source
  cubes — genuinely exist.
- The `ServiceAccount` identity entity has `name: String` and `namespace:
  k8s::Namespace`, so binding it to a string ref (`serviceAccountRef`,
  `serviceAccountName`) is expressible — the same name-binding pattern the
  catalogue already uses for resources (`secretRefs.contains(resource.name)`).
- The `impersonate` action exists; its resource is
  `io::k8s::authentication::serviceaccounts` (not the catch-all `k8s::Resource`),
  a *distinct* entity from the `k8s::ServiceAccount` principal, so #1 binds by
  name/namespace like the other two — the same shape, **not** simpler (correction,
  Finding 2).
- **Soundness is preserved for free**: the lowering desugars to the existing
  transition primitive, so the per-path `path ⟹ policy` solver check is
  unchanged, and the assumed identity's permission must be a real source cube. A
  mis-written binding yields `NOT SOUND` (caught), never a silent false grant.
- **Backward compatible**: default-identity mode is bit-for-bit today's
  behavior.

## Risks / open questions

1. **Mode shape** — R1 (per-source optional) vs R2 (literal all-or-nothing +
   sentinel). Recommend R1. *Decision needed.*
2. **Combinatorial blow-up** — naive lowering is impersonation × every `(a,r)` an
   identity can act in (~17 actions × many resources). Mitigation: generate a
   lowered transition **only for `(a,r)` where the input policy set has an
   `assume`-principal source cube** — exactly the info the `cubes` pass already
   produces (`escalate` filters `sources … c.env == env` and `continue`s on an
   empty source). Compute it inside the tool rather than materializing a huge
   `transitions.json`. *Caveat (Finding 5):* cost is not only transition count —
   each lowered transition re-runs `extend_schema` + `allow_cubes` + a per-path
   solver check **per acting principal type**. Phase 3 must set an explicit
   solver-cost budget/cap and dedupe identical `(a,r)` targets across acting
   principal types.
3. **Name ↔ entity binding** (verified field types, Finding 4):
   - **create-pods chain typechecks as-is, no projection** — `principal{i}.name ==
     resource1.serviceAccountRef` is `String == String`; `principal{i}.namespace ==
     resource1.namespace` is `k8s::Namespace == k8s::Namespace`. Better than feared.
   - **#1 impersonate** crosses `k8s::Namespace` (principal) vs `String` (the authn
     resource's namespace), so project: `principal{i}.namespace.name == resource.namespace`.
   - **#2 token-Secret is gated on derived fields that do not exist yet**:
     `core::secrets` today has neither `serviceAccountName` nor `type`. #2's `when`
     (both `principal{i}.name == resource1.serviceAccountName` and `resource1.type
     == "kubernetes.io/service-account-token"`) **cannot even be written/typecheck**
     until a pre-Phase-2 step adds both derived fields (the gap-#2 followup covers
     `serviceAccountName`; `type` is additional). A dependency, not a design flaw.
4. **Multi-hop** — this captures the assumed identity's **explicit** cubes, not its
   own **implicit** ones (`P → S`, then `S create pods → S get secret`). That needs
   a fixpoint (feed synthesized policies back as input). **Out of scope**; flag in
   the mechanism docs *and* emit a one-line guard in the tool output that the
   result is single-hop, so a consumer does not mistake it for transitive closure.
5. **Entity-typed `principal{i}` context attribute — CONFIRMED supported** (Finding
   3, Risk downgraded): `extend_schema` already adds `resource{i}` as an
   entity-typed context attr via a fresh common-type alias in the entity's defining
   namespace (golden `testdata/schema/target-context-other-namespace`). Mirror it
   exactly for `principal{i}`, aliasing in the **principal** entity type's namespace
   (`k8s`), and emit `principal{i}` from `context_attrs()`/`added_attrs()` so
   `eliminate` (`mentions_context_attrs`) drops it.
6. **Where the identity's cubes come from** — the input policy set must include
   the assumed identity's RBAC (policies with `principal is k8s::ServiceAccount`
   or a concrete SA). The catalogue examples will need a small SA-principal
   example policy to demonstrate a non-vacuous path.

## Phasing

- **Phase 1 (this): plan + feasibility review.** ← deliverable.
- Phase 2: Layer A core (per-source principal, `principal{i}` schema extension,
  escalate + eliminate), with golden tests incl. a principal-changing case and a
  regression that existing transitions are unchanged.
- Phase 3: Layer B lowering + `impersonation-transitions.json`, with pruning.
- Phase 4: the three k8s mechanisms as catalogue entries (`impersonate-assumes-sa`,
  `read-token-secret-assumes-sa`, `create-pod-assumes-sa`), each `[sound]`, e2e
  on kind, graph (a new `⟹ Identity` edge class), and close `KUBEHOUND-GAP.md`
  #1/#2.

## Deliberately not done (phase 1)

No code. No fixpoint multi-hop. `users`/`groups` identities beyond the SA pattern
(same shape, deferred). No change to the escalate algorithm's soundness machinery
— by design, the whole point is that it is reused unchanged.

## Feasibility review (independent, blind — folded in)

A fresh-context reviewer audited this plan against the actual source
(`transition.rs`, `escalate.rs`, `schema.rs`, `cubes.rs`, `expr.rs`) and
`k8s.cedarschema` (crate confirmed to compile, so the reading is against a valid
tree). **Verdict: FEASIBLE-WITH-CHANGES** — the two-layer architecture is sound
and the central soundness claim holds (the assumed identity's permission is a
*real source cube* under the unchanged per-path solver check, so a mis-written
binding yields `NOT SOUND`, never a silent false grant). Two blockers it caught,
now corrected above:

- **Blocker 1 (Finding 1):** `rename_vars` (`src/expr.rs`) does **not** remap
  `Var::Principal`; today that is correct (one shared principal), but under Layer A
  a source carrying the *assumed* principal would leave its cube's `principal.*`
  guards bound to the *acting* principal. Layer A must extend `rename_vars` to
  remap `Var::Principal → context.principal{i}`, and `principal_types()` must
  intersect over default-identity sources only. Corrected in Layer A and the
  Background note — the *soundness machinery* is unchanged, but `rename_vars` /
  the combination / `principal_types()` are not.
- **Blocker 2 (Finding 2):** `impersonate`'s resource is
  `io::k8s::authentication::serviceaccounts` (a distinct entity from the
  `k8s::ServiceAccount` principal), not `k8s::Resource`; `context.principalX ==
  resource` is a cross-type comparison that does not typecheck. #1 binds by
  name/namespace like the others. Corrected in the worked-examples table and the
  Feasibility summary.

Should-fixes (folded into Risks): the create-pods binding typechecks with no
projection; #2 is gated on *two* not-yet-existing `core::secrets` derived fields
(`serviceAccountName` **and** `type`); #1 needs a `k8s::Namespace.name` projection;
entity-typed `principal{i}` is confirmed supported (mirror `resource{i}`); set a
solver-cost budget for the Phase-3 lowering; emit a single-hop guard in the output.
Reviewer confirmed OK: multi-hop scope-out, the pruning mechanism, and that
`principals_for_action`/`request_envs` are real `cedar_policy::Schema` methods the
plan relies on.
