// == cubes ==
// (principal: Account, action: Action::"read", resource: Secret)
// policy1.cube0 [True]
@woodpecker("cube policy1.cube0")
permit(
  principal is Account,
  action == Action::"read",
  resource is Secret
);
// (principal: User, action: Action::"create", resource: Pod)
// policy0.cube0 [True]
@woodpecker("cube policy0.cube0")
permit(
  principal is User,
  action == Action::"create",
  resource is Pod
);
// == escalations ==
// create-pod-assumes-account: (principal: User, action: Action::"create", resource: Pod) via policy0.cube0; (principal: Account, action: Action::"read", resource: Secret) via policy1.cube0 -> (principal: User, action: Action::"read", resource: Secret) [sound]
// path: (context.action1 == Action::"create") && (context.action2 == Action::"read") && (context.principal2.name == context.resource1.runsAs) && (context.resource2 == resource)
@woodpecker("create-pod-assumes-account: policy0.cube0, policy1.cube0")
permit(
  principal is User,
  action == Action::"read",
  resource is Secret
);
