permit (
  principal is k8s::User,
  action == k8s::Action::"update",
  resource is apps::deployments_scale
);
