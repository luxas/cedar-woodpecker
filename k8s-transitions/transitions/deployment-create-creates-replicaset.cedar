permit (
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is apps::deployments
);
