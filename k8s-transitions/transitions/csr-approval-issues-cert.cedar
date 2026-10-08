// A user allowed to update the `approval` subresource of a
// CertificateSigningRequest. Writing the Approved condition drives the signer to
// issue a client certificate for the subject named in the CSR, so approval
// authority implies control over the parent CSR object (and the identity it
// mints). CSRs are cluster-scoped, so there is no namespace to match on.
permit (
  principal is k8s::User,
  action == k8s::Action::"update",
  resource is io::k8s::certificates::certificatesigningrequests_approval
);
