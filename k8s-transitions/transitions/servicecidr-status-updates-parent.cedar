// A user who may update ServiceCIDR status (cluster-scoped networking subresource).
permit (
  principal is k8s::User,
  action == k8s::Action::"update",
  resource is io::k8s::networking::servicecidrs_status
);
