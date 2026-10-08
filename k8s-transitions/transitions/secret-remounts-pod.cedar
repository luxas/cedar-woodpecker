// A user granted patch on secrets in namespace "team-a".
permit (
  principal is k8s::User,
  action == k8s::Action::"patch",
  resource is core::secrets
)
when { resource.namespace.name == "team-a" };
