// A user may create pods.
permit(principal is User, action == Action::"create", resource is Pod);
// Any account may read secrets (the identity whose permission is assumed).
permit(principal is Account, action == Action::"read", resource is Secret);
