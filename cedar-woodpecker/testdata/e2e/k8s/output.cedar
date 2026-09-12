// == cubes ==
// (principal: User, action: Action::"create", resource: Pod)
// policy1.cube0 [True, False, Error]
@woodpecker("cube policy1.cube0")
permit(
  principal is User,
  action == Action::"create",
  resource is Pod
) when {
  (principal.name == "alice") && principal.groups.contains("engineers") && (resource.namespace == "foo")
};
// (principal: User, action: Action::"create", resource: Secret)
// policy2.cube1 [True, False, Error]
@woodpecker("cube policy2.cube1")
permit(
  principal is User,
  action == Action::"create",
  resource is Secret
) when {
  principal.groups.contains("lucas-not-in-group")
};
// (principal: User, action: Action::"patch", resource: Pod)
// policy0.cube2 [True, False, Error]
@woodpecker("cube policy0.cube2")
permit(
  principal is User,
  action == Action::"patch",
  resource is Pod
) when {
  principal.groups.contains("engineers")
};
// == escalations ==
// pod-create-reads-secrets: (principal: User, action: Action::"create", resource: Pod) via policy1.cube0 -> (principal: User, action: Action::"get", resource: Secret) [sound]
// path: (context.action1 == Action::"create") && (principal.name == "alice") && principal.groups.contains("engineers") && (context.resource1.namespace == "foo") && (context.resource1.namespace == resource.namespace)
@woodpecker("pod-create-reads-secrets: policy1.cube0")
permit(
  principal is User,
  action == Action::"get",
  resource is Secret
) when {
  (principal.name == "alice") && (principal.groups.contains("engineers") && (resource.namespace == "foo"))
};
