// A user (e.g. a CSI attacher principal) who may update VolumeAttachment status
// (cluster-scoped storage subresource).
permit (
  principal is k8s::User,
  action == k8s::Action::"update",
  resource is io::k8s::storage::volumeattachments_status
);
