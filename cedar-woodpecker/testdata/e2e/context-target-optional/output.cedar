// == cubes ==
// (principal: User, action: Action::"create", resource: Pod)
// policy0.cube0 [True, False, Error]
@woodpecker("cube policy0.cube0")
permit(
  principal is User,
  action == Action::"create",
  resource is Pod
) when {
  (principal.name == "alice") && (!(context.timestamp <= 100))
};
// == escalations ==
// pod-create-reads-secrets: (principal: User, action: Action::"create", resource: Pod) via policy0.cube0 -> (principal: User, action: Action::"get", resource: Secret) [sound]
// path: (context.action1 == Action::"create") && (principal.name == "alice") && (!(context.context1.timestamp <= 100)) && (context has timestamp) && (context.context1.timestamp == context.timestamp)
@woodpecker("pod-create-reads-secrets: policy0.cube0")
permit(
  principal is User,
  action == Action::"get",
  resource is Secret
) when {
  (context has timestamp) && ((principal.name == "alice") && (!(context.timestamp <= 100)))
};
