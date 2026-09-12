// == cubes ==
// (principal: User, action: Action::"create", resource: Pod)
// policy0.cube0 [True, False, Error]
@woodpecker("cube policy0.cube0")
permit(
  principal is User,
  action == Action::"create",
  resource is Pod
) when {
  principal.groups.contains("eng") && resource.labels.contains("prod")
};
// == escalations ==
// pod-labels-reach-secrets: (principal: User, action: Action::"create", resource: Pod) via policy0.cube0 -> (principal: User, action: Action::"get", resource: Secret) [sound]
// path: (context.action1 == Action::"create") && principal.groups.contains("eng") && context.resource1.labels.contains("prod") && resource.labels.containsAll(context.resource1.labels)
@woodpecker("pod-labels-reach-secrets: policy0.cube0")
permit(
  principal is User,
  action == Action::"get",
  resource is Secret
) when {
  principal.groups.contains("eng") && resource.labels.contains("prod")
};
