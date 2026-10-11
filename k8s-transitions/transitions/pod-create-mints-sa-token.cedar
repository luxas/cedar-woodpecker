// A user allowed to create Pods. A Pod's projected serviceAccountToken volume
// causes the kubelet to call the TokenRequest API and mint a token for the Pod's
// ServiceAccount, so creating a Pod implies create-token on that SA.
permit (
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is core::pods
);
