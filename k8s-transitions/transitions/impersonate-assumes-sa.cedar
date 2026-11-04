// Alice (a User) may impersonate ServiceAccounts.
permit (
  principal is k8s::User,
  action == k8s::Action::"impersonate",
  resource is io::k8s::authentication::serviceaccounts
);
// A ServiceAccount may get ConfigMaps -- a representative permission Alice
// inherits by assuming the SA (any permission the SA holds would do).
permit (
  principal is k8s::ServiceAccount,
  action == k8s::Action::"get",
  resource is core::configmaps
);
