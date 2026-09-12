// == cubes ==
// (principal: User, action: Action::"create", resource: Pod)
// policy0.cube0 [True, False, Error]
@woodpecker("cube policy0.cube0")
permit(
  principal is User,
  action == Action::"create",
  resource is Pod
) when {
  (principal.name == "alice") && (context.tenant == "acme")
};
// == escalations ==
// pod-create-reads-logs: (principal: User, action: Action::"create", resource: Pod) via policy0.cube0 -> (principal: User, action: Audit::Action::"read", resource: Audit::Log) [sound]
// path: (principal.name == "alice") && (context.context1.tenant == "acme") && (resource.tenant == context.context1.tenant)
@woodpecker("pod-create-reads-logs: policy0.cube0")
permit(
  principal is User,
  action == Audit::Action::"read",
  resource is Audit::Log
) when {
  (principal.name == "alice") && (resource.tenant == "acme")
};
