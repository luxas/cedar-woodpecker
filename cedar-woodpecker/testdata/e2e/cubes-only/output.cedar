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
