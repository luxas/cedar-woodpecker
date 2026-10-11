// A user allowed to create RoleBindings. Creating a RoleBinding binds a Role to
// a subject, which is exactly the `bind` permission RBAC requires to grant that
// Role's rules to another principal.
permit (
  principal is k8s::User,
  action == k8s::Action::"create",
  resource is io::k8s::authorization::rbac::rolebindings
);
