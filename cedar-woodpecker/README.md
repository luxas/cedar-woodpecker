# cedar-woodpecker

Woodpecker finds *privilege-escalation paths* in a Cedar policy set and synthesizes the
policies they imply. Levaillant's woodpecker lives in the cedar forests of North Africa; a cedar
is a dense tree with good defenses, yet the woodpecker punches holes in it.

It is the Phase 4 tool of the symbolic Cedar evaluator (see
`cedar-policy-symcc/src/evaluator/README.md`, "Phase 4: Existential quantifier elimination").
Experimental; the crate is not published.

## What it does

1. **Explicit permissions.** The policy set becomes allow-only DNF cubes (`dnf::allow_cubes`:
   forbids folded into the permits as deny witnesses, every permit split into the cubes of its
   DNF), typechecked once in every request environment, constant-folded with TPE and folded with
   the symbolic evaluator under the schema's `@semantics` assumptions and the given entity data.
   Every cube that can still be `true` in an environment is a *source cube* there
   (`cedar-woodpecker cubes`, printed as one policy per cube over its environment).
2. **Transitions.** A transition function `(pt, [(at_i, rt_i)], at', rt', T)` says that a
   principal of type `pt` holding permissions in the source environments `(pt, at_i, rt_i)`
   implicitly holds one in the target environment `(pt, at', rt')`, under the condition `T`.
   For each transition the schema is copied with the target action's context extended by the
   intermediate requests — required attributes `action{i}`, `resource{i}` and `context{i}`, the
   last of the source action's context type, each typed through a fresh common type added to
   the namespace that defines what it names, so that every name resolves where it was written —
   and the source cubes, with
   `action`/`resource`/`context` renamed to `context.action{i}`/`context.resource{i}`/
   `context.context{i}`, are disjoined per source, conjoined over the sources and with `T`. That condition, as a permit
   over the target environment, is split into cubes and folded with the evaluator again; every
   cube that can be `true` is an **escalation path** (`cedar-woodpecker escalate`).
3. **Synthesis.** From a path, the implied policy over the original schema is synthesized by
   eliminating the intermediate request. First the path's equalities are propagated
   (*equivalence classes*, plan 16): a conjunct `<expr> == <constant>` substitutes the constant
   for `<expr>` everywhere else (a record literal is simplified under `.a`/`has a`), the error
   guards `<expr> == <expr>` are dropped, an expression free of the intermediate request is
   canonical for one that mentions it, and among two that mention it the first is. Then two
   set operations over the same intermediate set (or element) become one over what remains,
   by the 81-cell matrix of plan 13 (`cedar-lean/Cedar/Thm/DNF/SetQE.lean` in cedar-spec), best rewrite
   first; before that, all `in`s over the same intermediate entity give every fact transitivity
   implies (`a in X ∧ X in c ⇒ a in c`, and the negated forms where the moved operand is
   syntactically an entity — an attribute may be a set, which `in` rejects on its left). Then every
   conjunct still mentioning `context.action{i}`/`context.resource{i}`/`context.context{i}`
   becomes `true` — the over-approximation the long bounds will refine — and the `has`/`hasTag`
   guards come first, so an access the classes moved stays guarded. Every synthesized policy is
   validated against the original schema (a failure is reported for that path as `NOT VALID`,
   not as an abort) and checked *sound* — the path, assumed, implies it — and reported with its
   provenance.

## Usage

```sh
cedar-woodpecker cubes    --schema k8s.cedarschema --policies k8s.cedar [--entities e.json] [--json]
cedar-woodpecker escalate --schema k8s.cedarschema --policies k8s.cedar --transitions t.json [--entities e.json] [--json]
```

The policies come from `--policies` or stdin; the schema is JSON with a `.json` extension,
Cedar syntax otherwise. Needs `cvc5` on `PATH` (or in `CVC5`).

### Transition file

```json
{ "transitions": [ {
    "name": "pod-create-reads-secrets",
    "principal": ["User"],
    "sources": [ { "action": "Action::\"create\"", "resource": "Pod" } ],
    "target":  { "action": "Action::\"get\"", "resource": "Secret" },
    "when": "context.resource1.namespace == resource.namespace"
} ] }
```

`principal` is optional (default: every principal type applicable to the target action and to
every source action). `when` is a Cedar expression over `principal`, `action`, `resource`,
`context` (the target request) and `context.action{i}`/`context.resource{i}`/`context.context{i}`
(the `i`-th source request, 1-based; `context{i}` has the type the schema gives the source
action's context, an empty record when it declares none); it defaults to `true`. A fact a
source policy states about its context (`context.tenant == "acme"`) reaches the target only
through `when` (`context.context1.tenant == resource.tenant`); otherwise it becomes `true`.
`when` is typechecked on its own in the target environment, so it must guard an optional
attribute itself (`context.context1 has reason && context.context1.reason == …`) rather than
rely on the source policies' guards.

### Example

With the policies of `tests/k8s.cedar` — engineers patch Pods; alice, when an engineer,
creates Pods in the `foo` namespace; nobody creates Pods in `kube-system` — and the transition
above ("whoever can create a Pod can read the Secrets of its namespace"):

```
// pod-create-reads-secrets: (principal: User, action: Action::"create", resource: Pod) via policy1.cube0 -> (principal: User, action: Action::"get", resource: Secret) [sound]
// path: …
@woodpecker("pod-create-reads-secrets: policy1.cube0")
permit(
  principal is User,
  action == Action::"get",
  resource is Secret
) when {
  principal.name == "alice" && principal.groups.contains("engineers") && resource.namespace == "foo"
};
```

The namespace restriction, stated about the intermediate Pod
(`context.resource1.namespace == "foo"`), reaches the Secret through the transition's
`context.resource1.namespace == resource.namespace` — the equivalence classes at work.

## Library

`cedar_woodpecker::{LoadedSchema, parse_policies, evaluator_with, source_cubes, Transition,
extend_schema, escalate}` are the pieces the CLI composes; `equiv::rewrite` is the
equivalence-class rewrite and `escalate::eliminate` the elimination step the later
revisions refine.

## Tests

Every test is a case directory under `testdata/<suite>/<case>/` with its full inputs and its
expected output, run by `tests/golden.rs`:

| suite | inputs | output |
|---|---|---|
| `e2e` | `schema.cedarschema`, `input.cedar`, optional `transitions.json`, optional `entities.json` | `output.cedar` (the cubes as policies, then the synthesized policies, with comments — or the error) and `output.json` |
| `equiv` | `input.txt`: `removed: <attrs>`, then one conjunct per line | `expected.txt`: the rewritten conjuncts |
| `sets` | the same | `expected.txt`: the conjuncts after the set and `in` rewrites — one case per matrix cell (`01-e-e` … `81-nm-nm`), the `in` rules, the pair preferences |
| `expr` | `input.txt`: `op: rename\|simplify\|replace\|mentions\|constant\|and_chain\|or_chain` (+ `from`/`to`/`removed`), then the expressions | `expected.txt` |
| `schema` | `schema.cedarschema`, `transition.json` | `expected.cedarschema` (the extended schema) or `expected.txt` (the error) |
| `transition` | `transitions.json` | `expected.txt`: the listing, or the error |

`UPDATE_GOLDEN=1 cargo test -p cedar-woodpecker` rewrites the expectations; review the diff
like code. The `e2e` suite needs `cvc5`.

