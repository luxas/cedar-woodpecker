// alice creates Pods labelled prod
permit(principal, action == Action::"create", resource is Pod)
when { principal.name == "alice" && context.labels.contains("prod") };
