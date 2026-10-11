// A user granted get on the pods/ephemeralcontainers subresource in namespace "team-a".
permit (
  principal is k8s::User,
  action == k8s::Action::"get",
  resource is core::pods_ephemeralcontainers
)
when { resource.namespace.name == "team-a" };
