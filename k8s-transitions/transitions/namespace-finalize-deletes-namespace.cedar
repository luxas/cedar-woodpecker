// A controller/user permitted to write the /finalize subresource of namespaces.
permit (
  principal is k8s::User,
  action == k8s::Action::"update",
  resource is core::namespaces_finalize
);
