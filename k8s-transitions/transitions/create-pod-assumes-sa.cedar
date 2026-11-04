// Alice (a User) may create Pods.
permit (
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is core::pods
);
// A ServiceAccount may get ConfigMaps -- a representative permission Alice
// inherits by running a Pod as that SA and replaying its projected token.
permit (
  principal is k8s::ServiceAccount,
  action == k8s::Action::"get",
  resource is core::configmaps
);
