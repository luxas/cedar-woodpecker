// Alice may update/patch/get the ephemeralcontainers subresource of Pods.
permit (
  principal is k8s::User,
  action in [k8s::Action::"update", k8s::Action::"patch", k8s::Action::"get"],
  resource is core::pods_ephemeralcontainers
);
