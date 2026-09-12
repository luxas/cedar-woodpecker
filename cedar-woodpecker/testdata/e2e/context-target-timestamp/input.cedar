// alice creates Pods after the epoch second 100, giving a reason
permit(principal, action == Action::"create", resource is Pod)
when { principal.name == "alice" && context.timestamp > 100 && context has reason };
