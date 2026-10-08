// Alice may create Pods in any namespace.
permit (
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is core::pods
);
