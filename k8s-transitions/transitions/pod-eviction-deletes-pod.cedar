// Alice may create eviction subresources on Pods in any namespace.
permit (
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is core::pods_eviction
);
