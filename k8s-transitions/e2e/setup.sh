#!/usr/bin/env bash
# Create (or reuse) the kind cluster for the implicit-permission e2e suite and
# preload the busybox image onto the node. Idempotent.
set -euo pipefail
cd "$(dirname "$0")"
export PATH="$HOME/.local/bin:$PATH"
CLUSTER="${WP_E2E_CLUSTER:-wp-e2e}"
IMG="${WP_E2E_IMAGE:-busybox:1.36}"

need() { command -v "$1" >/dev/null 2>&1 || { echo "missing required tool: $1" >&2; exit 2; }; }
need kind; need kubectl; need docker

if kind get clusters 2>/dev/null | grep -qx "$CLUSTER"; then
  echo "cluster '$CLUSTER' already exists; reusing"
else
  echo "creating kind cluster '$CLUSTER' ..."
  kind create cluster --name "$CLUSTER" --config kind-cluster.yaml --wait 180s
fi

echo "preloading image $IMG ..."
docker image inspect "$IMG" >/dev/null 2>&1 || docker pull "$IMG"
kind load docker-image "$IMG" --name "$CLUSTER" >/dev/null 2>&1 || true

kubectl --context "kind-$CLUSTER" wait --for=condition=Ready node --all --timeout=120s
echo "ready. context: kind-$CLUSTER"
