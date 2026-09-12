// alice creates Pods when she gives the namespace as the reason
permit(principal, action == Action::"create", resource is Pod)
when { principal.name == "alice" && context has reason && context.reason == resource.namespace };

// bob creates Pods, no reason given
permit(principal, action == Action::"create", resource is Pod)
when { principal.name == "bob" };
