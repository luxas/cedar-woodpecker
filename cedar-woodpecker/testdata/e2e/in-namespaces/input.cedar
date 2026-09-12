// Pods may be created under the foo namespace
permit(principal, action == Action::"create", resource is Pod)
when { resource.ns in Namespace::"foo" };
