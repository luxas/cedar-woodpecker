// alice creates Pods, not as a dry run
permit(principal, action == Action::"create", resource is Pod)
when { principal.name == "alice" && context.dryRun == false && resource.namespace == "foo" };
