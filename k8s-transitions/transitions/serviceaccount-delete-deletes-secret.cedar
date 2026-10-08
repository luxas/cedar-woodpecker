// A user allowed to delete ServiceAccounts. The ServiceAccount token controller /
// legacy token cleaner garbage-collects the Secrets that hold the SA's
// auto-generated tokens, so deleting a ServiceAccount causes those Secrets to be
// deleted as a side effect.
permit (
  principal is k8s::User,
  action == k8s::Action::"delete",
  resource is core::serviceaccounts
);
