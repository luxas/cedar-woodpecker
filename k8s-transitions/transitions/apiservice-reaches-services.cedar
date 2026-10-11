// A user who may register aggregated APIs (cluster-scoped APIService objects).
permit (
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is io::k8s::apiregistration::apiservices
);
