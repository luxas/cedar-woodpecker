// A user who may create Services in namespace "shop".
permit (
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is core::services
)
when { resource.namespace.name == "shop" };
