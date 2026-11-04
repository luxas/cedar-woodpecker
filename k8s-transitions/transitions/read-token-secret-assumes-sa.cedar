// Alice (a User) may get Secrets.
permit (
  principal is k8s::User,
  action == k8s::Action::"get",
  resource is core::secrets
);
// A ServiceAccount may get ConfigMaps -- a representative permission Alice
// inherits by reading the SA's token Secret and replaying its bearer token.
permit (
  principal is k8s::ServiceAccount,
  action == k8s::Action::"get",
  resource is core::configmaps
);
