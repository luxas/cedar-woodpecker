#!/usr/bin/env bash
# Implicit permission (claimed): create APIService -> get/watch Service
#                                            (apiservice-reaches-services)
#
# HONEST SKIP. Holding `create apiservices` lets a principal register an
# aggregated API whose `spec.service` names a backing Service in any namespace;
# the kube-aggregator then GETs and WATCHes that Service (via its ServiceResolver
# and shared Service lister) to resolve its ClusterIP/endpoints and PROXY API
# requests to it. The observable implicit action `q` is "the aggregator reads
# and watches the backing Service" -- and the only honest way to observe it is
# to drive a request through the aggregation proxy path to the registered
# Service and see the aggregator resolve it.
#
# That path cannot be exercised on a vanilla single-node kind cluster:
#   - There is no real aggregated apiserver backend here. A standalone Service
#     with a selector is NOT an extension-apiserver; it does not serve the
#     discovery/aggregation handshake. Registering an APIService with
#     spec.service pointing at an ordinary Service yields an APIService whose
#     `Available` condition goes False (FailedDiscoveryCheck), because the
#     aggregator probes the backend and finds no apiserver there.
#   - The aggregator's get/watch of the backing Service happens with the
#     *aggregator's own* credentials inside the apiserver process; there is no
#     controller-written status field on an ordinary Service, nor an audit
#     surface reachable here, that attributes a specific get/watch to the
#     APIService we registered. (Contrast flowschema-references-prioritylevel,
#     whose APF controller DOES write a resolvable Dangling status we can read.)
#   - Standing up a genuine extension apiserver (its Deployment, Service, serving
#     certs, and the APIService registration) is exactly the "needs an aggregated
#     apiserver" case CONTRIBUTING names as a legitimate SKIP, and would also
#     mutate shared cluster state well beyond `e2e/tests/`.
#
# The transition's own modeling notes agree the backing Service is named purely
# in the APIService spec with no derived back-reference (hence `when "true"`, an
# over-approximation). We therefore SKIP with the precise reason rather than fake
# a get/watch we cannot attribute.
#
# Spec: ../transitions/apiservice-reaches-services.{md,json}  (when: true)
source "$(dirname "$0")/../lib.sh"
test_start "apiservice-reaches-services"

echo "RESULT: SKIP apiservice-reaches-services the aggregator's get/watch of the backing Service requires a real aggregated/extension apiserver behind the APIService (vanilla single-node kind has none; an ordinary Service is not an apiserver and the APIService goes Available=False/FailedDiscoveryCheck), and the aggregator's read runs with its own credentials inside the apiserver with no status/audit surface here that attributes a specific get/watch to the registered APIService -- unprovable without standing up an extension apiserver and mutating shared cluster state."
exit 3
