// A user granted update on the pods/status subresource in namespace "team-a".
permit (
  principal is k8s::User,
  action == k8s::Action::"update",
  resource is core::pods_status
)
when { resource.namespace.name == "team-a" };
