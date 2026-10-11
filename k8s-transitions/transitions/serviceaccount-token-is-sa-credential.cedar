// A user allowed to create a token (the serviceaccounts/token TokenRequest
// subresource) for a ServiceAccount. The minted bearer token IS that SA's
// identity, so holding this permission is at least as strong as being able to
// read / act as the ServiceAccount itself.
permit (
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is core::serviceaccounts_token
);
