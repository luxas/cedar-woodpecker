// A user granted get on the pods/status subresource in namespace "team-a".
permit (
  principal is k8s::User,
  action == k8s::Action::"get",
  resource is core::pods_status
)
when { resource.namespace.name == "team-a" };
