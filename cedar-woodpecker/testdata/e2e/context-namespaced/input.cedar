// alice creates Pods for the acme tenant
permit(principal, action == Action::"create", resource is Pod)
when { principal.name == "alice" && context.tenant == "acme" };
