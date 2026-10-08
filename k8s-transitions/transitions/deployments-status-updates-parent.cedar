// A user granted update on the deployments/status subresource in namespace "team-a".
permit (
  principal is k8s::User,
  action == k8s::Action::"update",
  resource is apps::deployments_status
)
when { resource.namespace.name == "team-a" };
