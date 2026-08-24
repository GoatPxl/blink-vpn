Test-only RSA key material.

`test_rsa_key.pem` is a throwaway 2048-bit RSA key generated for the GCE adapter's unit tests. It
signs JWTs against a mockito server that accepts any assertion, and it authenticates nothing: no
Google project has ever seen the matching public key.

It exists because the service-account flow cannot be exercised without a structurally valid PEM —
`EncodingKey::from_rsa_pem` parses the key before any test assertion is reached.

Do not reuse it for anything, and do not treat a key in this directory as a finding. Real
credentials never live in the repo (AGENTS.md 6); they live in Windows Credential Manager.
