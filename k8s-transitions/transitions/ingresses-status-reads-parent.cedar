// A user granted get on the ingresses/status subresource in namespace "team-a".
permit (
  principal is k8s::User,
  action == k8s::Action::"get",
  resource is io::k8s::networking::ingresses_status
)
when { resource.namespace.name == "team-a" };
