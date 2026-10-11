// A user who may create StorageClasses (cluster-scoped).
permit (
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is io::k8s::storage::storageclasses
);
