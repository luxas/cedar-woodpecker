// A user who may delete namespaces (e.g. a tenant admin over "team-a").
permit (
  principal is k8s::User,
  action == k8s::Action::"delete",
  resource is core::namespaces
)
when { resource.name == "team-a" };
