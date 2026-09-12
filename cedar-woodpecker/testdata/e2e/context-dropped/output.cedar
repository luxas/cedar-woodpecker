// == cubes ==
// (principal: User, action: Action::"create", resource: Pod)
// policy0.cube0 [True, False, Error]
@woodpecker("cube policy0.cube0")
permit(
  principal is User,
  action == Action::"create",
  resource is Pod
) when {
  (principal.name == "alice") && (context.dryRun == false) && (resource.namespace == "foo")
};
// == escalations ==
// pod-create-reads-secrets: (principal: User, action: Action::"create", resource: Pod) via policy0.cube0 -> (principal: User, action: Action::"get", resource: Secret) [sound]
// path: (context.action1 == Action::"create") && (principal.name == "alice") && (context.context1.dryRun == false) && (context.resource1.namespace == "foo") && (context.resource1.namespace == resource.namespace)
@woodpecker("pod-create-reads-secrets: policy0.cube0")
permit(
  principal is User,
  action == Action::"get",
  resource is Secret
) when {
  (principal.name == "alice") && (resource.namespace == "foo")
};
