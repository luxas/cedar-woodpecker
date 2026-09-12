// engineers can patch Pods
permit(principal, action == Action::"patch", resource is Pod)
when { principal.groups.contains("engineers") };

// alice, when in the engineers group, can create Pods in the foo namespace
permit(principal, action == Action::"create", resource is Pod)
when { principal.name == "alice" && principal.groups.contains("engineers") && resource.namespace == "foo" };

// members of a group nobody is in can create Secrets
permit(principal, action == Action::"create", resource is Secret)
when { principal.groups.contains("lucas-not-in-group") };

// nobody creates Pods in kube-system
forbid(principal, action == Action::"create", resource is Pod)
when { resource.namespace == "kube-system" };
