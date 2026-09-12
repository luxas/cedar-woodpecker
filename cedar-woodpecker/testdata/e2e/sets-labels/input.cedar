// engineers create production-labelled Pods
permit(principal, action == Action::"create", resource is Pod)
when { principal.groups.contains("eng") && resource.labels.contains("prod") };
