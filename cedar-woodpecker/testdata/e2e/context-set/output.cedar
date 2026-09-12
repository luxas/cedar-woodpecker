// == cubes ==
// (principal: User, action: Action::"create", resource: Pod)
// policy0.cube0 [True, False, Error]
@woodpecker("cube policy0.cube0")
permit(
  principal is User,
  action == Action::"create",
  resource is Pod
) when {
  (principal.name == "alice") && context.labels.contains("prod")
};
// == escalations ==
// pod-create-reads-secrets: (principal: User, action: Action::"create", resource: Pod) via policy0.cube0 -> (principal: User, action: Action::"get", resource: Secret) [sound]
// path: (context.action1 == Action::"create") && (principal.name == "alice") && context.context1.labels.contains("prod") && (context.context1.labels == resource.labels)
@woodpecker("pod-create-reads-secrets: policy0.cube0")
permit(
  principal is User,
  action == Action::"get",
  resource is Secret
) when {
  (principal.name == "alice") && resource.labels.contains("prod")
};
