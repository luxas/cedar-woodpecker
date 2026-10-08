// A node agent / operator may delete the Node object for the node it runs on.
permit(principal is k8s::User, action == k8s::Action::"delete", resource is core::nodes)
when { resource.name == "worker-1" };
