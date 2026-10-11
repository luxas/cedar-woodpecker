// A user who may create FlowSchemas (cluster-scoped APF config).
permit (
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is io::k8s::apiserver::flowcontrol::flowschemas
);
