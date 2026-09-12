// == cubes ==
// (principal: User, action: Action::"create", resource: Pod)
// policy0.cube0 [True, False, Error]
@woodpecker("cube policy0.cube0")
permit(
  principal is User,
  action == Action::"create",
  resource is Pod
) when {
  resource.ns in Namespace::"foo"
};
// == escalations ==
// pod-namespace-reaches-secrets: (principal: User, action: Action::"create", resource: Pod) via policy0.cube0 -> (principal: User, action: Action::"get", resource: Secret) [sound]
// path: (context.action1 == Action::"create") && (context.resource1.ns in Namespace::"foo") && (resource.ns in context.resource1.ns)
@woodpecker("pod-namespace-reaches-secrets: policy0.cube0")
permit(
  principal is User,
  action == Action::"get",
  resource is Secret
) when {
  resource.ns in Namespace::"foo"
};
