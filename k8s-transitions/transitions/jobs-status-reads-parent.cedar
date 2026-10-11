// A user granted get on the jobs/status subresource in namespace "team-a".
permit (
  principal is k8s::User,
  action == k8s::Action::"get",
  resource is batch::jobs_status
)
when { resource.namespace.name == "team-a" };
