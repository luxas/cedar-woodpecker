#!/usr/bin/env python3
"""Generate the implicit-permission graph (permission cubes as nodes, transitions
as edges) from the catalogue in k8s-transitions/.

Nodes  = permission cubes: a (principal, action, resource) request environment.
         (every transition here uses principal k8s::User, so a node is keyed by
         (action, resource); the principal is carried for completeness.)
Edges  = transitions: a *source* cube implies a *target* cube ("holding source
         implicitly grants target"), labelled with the mechanism, its Cedar
         `when` condition, the paper severity, the WHY permalinks and the live
         e2e result (PASS / SKIP).

Outputs (both self-contained, no server needed to view):
  graph/graph.json  -- the graph data (reusable)
  graph/index.html  -- the interactive viewer with graph.json inlined

Run from anywhere:  python3 k8s-transitions/graph/generate.py
"""
from __future__ import annotations
import json, re, glob, os, collections, html

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)  # k8s-transitions/

SEV_RANK = {"Critical": 4, "High": 3, "Medium": 2, "Low": 1}


def load_transitions():
    return json.load(open(os.path.join(ROOT, "transitions.json")))["transitions"]


def load_e2e_status():
    """name -> {status, reason}. Reads the committed per-test RESULT line so each
    SKIP carries its reason. A node test the shared runner guards ('runs it
    isolated') is relabelled ISOLATED — it is proven, just run on its own."""
    out = {}
    summ = os.path.join(ROOT, "e2e", "results", "summary.json")
    if os.path.exists(summ):
        for r in json.load(open(summ))["results"]:
            out[r["name"]] = {"status": r["status"], "reason": ""}
    # enrich with the per-test RESULT line (status + reason text)
    for log in glob.glob(os.path.join(ROOT, "e2e", "results", "*.log")):
        name = os.path.basename(log)[:-4]
        try:
            for line in open(log):
                m = re.match(r"RESULT:\s+(PASS|SKIP|FAIL)\s+(\S+)\s*(.*)", line.strip())
                if m:
                    status, _, reason = m.group(1), m.group(2), m.group(3).strip()
                    rec = out.setdefault(name, {"status": status, "reason": ""})
                    rec["status"] = status
                    rec["reason"] = reason
                    if status == "SKIP" and "isolated" in reason.lower():
                        rec["status"] = "ISOLATED"
                    break
        except OSError:
            pass
    return out


def parse_md():
    """basename.json -> owning md metadata (md path, severity tokens, why links, title, statement)."""
    owner = {}
    md_meta = {}
    for md in glob.glob(os.path.join(ROOT, "transitions", "*.md")):
        txt = open(md).read()
        base = os.path.basename(md)
        owned = set(re.findall(r"([a-z0-9][a-z0-9-]+)\.json", txt))
        sev = set()
        for s in re.findall(r"severity \*\*([A-Za-z/]+)\*\*", txt):
            for part in s.split("/"):
                part = part.strip().capitalize()
                if part in SEV_RANK:
                    sev.add(part)
        why = re.findall(r"https://github\.com/kubernetes/kubernetes/blob/[0-9a-f]+/[^\s)]+", txt)
        title = ""
        m = re.search(r"^#\s+(.+)$", txt, re.M)
        if m:
            title = m.group(1).strip()
        md_meta[base] = {
            "md": base,
            "sev": sev,
            "why": list(dict.fromkeys(why))[:5],
            "title": title,
        }
        for o in owned:
            owner.setdefault(o, base)
    return owner, md_meta


def parse_index_severity():
    """md filename -> set of severity tokens, from the authoritative index README table."""
    p = os.path.join(ROOT, "transitions", "README.md")
    out = {}
    if not os.path.exists(p):
        return out
    txt = open(p).read()
    for md, sev in re.findall(r"\[`([^`]+\.md)`\][^|\n]*\|[^|\n]*\|\s*([A-Za-z/]+)\s*\|", txt):
        toks = {s.strip().capitalize() for s in sev.split("/") if s.strip().capitalize() in SEV_RANK}
        if toks:
            out[md] = toks
    return out


def owner_for(name, owner, md_meta):
    """Resolve the owning md for a transition name, with a longest-prefix fallback."""
    if name in owner:
        return owner[name]
    # fallback: the md filename sharing the longest hyphen-prefix with the name
    best, best_len = None, 0
    toks = name.split("-")
    for base in md_meta:
        stem = base[:-3]
        mt = stem.split("-")
        n = 0
        for a, b in zip(toks, mt):
            if a == b:
                n += 1
            else:
                break
        if n > best_len:
            best, best_len = base, n
    return best


VERB_RE = re.compile(r'Action::"([^"]+)"')


def parse_env(action, resource):
    vm = VERB_RE.search(action)
    verb = vm.group(1) if vm else action
    # group = API group for colouring
    if resource == "k8s::Resource":
        group = "all"
    elif resource.startswith("io::k8s::"):
        rest = resource[len("io::k8s::"):].split("::")
        group = rest[0] if rest else "io"
    else:
        group = resource.split("::")[0]
    leaf = resource.split("::")[-1]
    # subresource prettify: foo_status -> foo/status
    display = leaf.replace("_", "/")
    return verb, group, leaf, display


def main():
    ts = load_transitions()
    e2e = load_e2e_status()
    owner, md_meta = parse_md()
    idx_sev = parse_index_severity()

    nodes = {}
    edges = []

    def node_id(action, resource):
        verb, group, leaf, display = parse_env(action, resource)
        nid = f"{verb}:{resource}"
        if nid not in nodes:
            nodes[nid] = {
                "id": nid,
                "verb": verb,
                "resource": resource,
                "group": group,
                "label": f"{verb} {display}",
                "display": display,
                "indeg": 0,
                "outdeg": 0,
            }
        return nid

    for t in ts:
        name = t["name"]
        tgt = node_id(t["target"]["action"], t["target"]["resource"])
        base = owner_for(name, owner, md_meta)
        meta = md_meta.get(base, {})
        # severity: the index-table span is AUTHORITATIVE (acceptance: every edge's
        # severity matches transitions/README.md's table). Inline md-body tokens are
        # used ONLY as a fallback when the table has no row for this mechanism, so a
        # stale body token can never override or widen the table's severity.
        if base in idx_sev:
            sev_set = set(idx_sev[base])
        else:
            sev_set = set(meta.get("sev", set())) or {"Low"}
        sev_tokens = sorted(sev_set, key=lambda s: -SEV_RANK[s])
        top_sev = sev_tokens[0]
        status = e2e.get(name, {}).get("status", "UNKNOWN")
        status_reason = e2e.get(name, {}).get("reason", "")
        for s in t["sources"]:
            src = node_id(s["action"], s["resource"])
            edges.append({
                "id": name,
                "source": src,
                "target": tgt,
                "name": name,
                "title": meta.get("title", name),
                "when": t.get("when", "true"),
                "severity": top_sev,
                "severity_span": sev_tokens,
                "status": status,
                "status_reason": status_reason,
                "md": meta.get("md", ""),
                "why": meta.get("why", []),
            })
            nodes[src]["outdeg"] += 1
            nodes[tgt]["indeg"] += 1

    graph = {
        "meta": {
            "transitions": len(ts),
            "edges": len(edges),
            "nodes": len(nodes),
            "sources": "ImplicitPermission/.../trigger.txt (328 refs) -> 62 transitions",
            "paper": "Yang et al., Dangers Behind Access Control (CCS '25)",
            "repo_branch": "17-k8s-transitions",
            "schema": "k8s-transitions/k8s.cedarschema",
        },
        "nodes": sorted(nodes.values(), key=lambda n: (-(n["indeg"] + n["outdeg"]), n["label"])),
        "edges": sorted(edges, key=lambda e: (-SEV_RANK.get(e["severity"], 0), e["name"])),
    }

    out_json = os.path.join(HERE, "graph.json")
    with open(out_json, "w") as f:
        json.dump(graph, f, indent=1)
    print(f"wrote {out_json}: {len(nodes)} cubes, {len(edges)} implications")

    tmpl = open(os.path.join(HERE, "template.html")).read()
    data_blob = json.dumps(graph)
    page = tmpl.replace("/*__GRAPH_DATA__*/", data_blob)
    out_html = os.path.join(HERE, "index.html")
    with open(out_html, "w") as f:
        f.write(page)
    print(f"wrote {out_html}")

    # a small severity x status matrix for sanity
    mx = collections.Counter((e["severity"], e["status"]) for e in edges)
    for (sv, st), c in sorted(mx.items(), key=lambda kv: (-SEV_RANK.get(kv[0][0], 0), kv[0][1])):
        print(f"  {sv:9} {st:8} {c}")


if __name__ == "__main__":
    main()
