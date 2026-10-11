// Alice may update (and patch) Pods in any namespace.
permit (
  principal is k8s::User,
  action in [k8s::Action::"update", k8s::Action::"patch"],
  resource is core::pods
);
