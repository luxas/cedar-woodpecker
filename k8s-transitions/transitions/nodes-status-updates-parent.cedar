// A user granted update on the nodes/status subresource (cluster-scoped).
permit (
  principal is k8s::User,
  action == k8s::Action::"update",
  resource is core::nodes_status
);
