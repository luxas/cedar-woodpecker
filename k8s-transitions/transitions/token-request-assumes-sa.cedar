// Alice (a User) may create tokens for ServiceAccounts (the serviceaccounts/token
// TokenRequest subresource).
permit (
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is core::serviceaccounts_token
);
// A ServiceAccount may get ConfigMaps -- a representative permission Alice
// inherits by minting the SA's token and presenting it (any permission the SA
// holds would do).
permit (
  principal is k8s::ServiceAccount,
  action == k8s::Action::"get",
  resource is core::configmaps
);
