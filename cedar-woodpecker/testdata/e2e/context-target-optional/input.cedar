// alice creates Pods after the epoch second 100
permit(principal, action == Action::"create", resource is Pod)
when { principal.name == "alice" && context.timestamp > 100 };
