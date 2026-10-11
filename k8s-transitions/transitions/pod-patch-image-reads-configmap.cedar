// Alice may patch Pods in any namespace (e.g. to roll a container image).
permit (
  principal is k8s::User,
  action == k8s::Action::"patch",
  resource is core::pods
);
