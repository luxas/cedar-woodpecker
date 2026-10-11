#!/usr/bin/env bash
# Delete the kind cluster for the implicit-permission e2e suite.
set -uo pipefail
export PATH="$HOME/.local/bin:$PATH"
CLUSTER="${WP_E2E_CLUSTER:-wp-e2e}"
kind delete cluster --name "$CLUSTER"
