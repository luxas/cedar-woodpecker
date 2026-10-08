// A user who may create namespaces (e.g. a self-service onboarding role).
permit (
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is core::namespaces
);
