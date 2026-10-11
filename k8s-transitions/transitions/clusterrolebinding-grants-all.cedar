// A user allowed to create ClusterRoleBindings. A ClusterRoleBinding can bind
// the built-in cluster-admin ClusterRole to any subject, so this single explicit
// permission implies every verb on every resource cluster-wide.
permit (
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is io::k8s::authorization::rbac::clusterrolebindings
);
