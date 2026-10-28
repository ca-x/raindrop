# Vendored compatibility dependencies

These two libraries preserve DeepL's existing browser fingerprint while applying
upstream security fixes. No application protocol or certificate-verification
policy is replaced.

## Rustls 0.23.46

- Compatibility source: `apify/rustls` revision
  `5ce04170965aff5da23154e79f5ee25c37134da1`.
- Security source: `rustls/rustls` tag `v/0.23.46`, revision
  `f04804fa13341a9b1dc02b5d1dd8a746f2c4b7fb`.
- The complete upstream histories merge without conflicts. The result of
  `git merge-tree --write-tree` with these revisions is
  `75a71462dddeca20103275ecceab66c719f34474`.
- `rustls/src` and `rustls/build.rs` are copied without edits from that tree,
  excluding upstream's unpublished `src/testdata`. The standalone manifest
  resolves inherited workspace dependencies and excludes development targets.
  The package version comes from upstream, not a local version-only bump.
- The merge includes the TLS encryption-level boundary check, bounded Brotli
  certificate decoding, exact compressed-message consumption, session-secret
  redaction, ECH rejection ordering, and authenticated key-exchange hints.
- Keep the `impit` fingerprint APIs and the application-level explicit ring
  provider. Do not replace this crate with an unmodified upstream release until
  the fingerprint consumer supports it.
- Original Apache-2.0, ISC, and MIT license files are retained. The MIT option
  is reproduced in `THIRD_PARTY_NOTICES` for packaged distributions.

## dlx 3.0.1

- Source: `hominsu/deeplx-rs` revision
  `03054aebd09b54e9934642cd8f11f212e2513065`, `crates/dlx/src`.
- Library source is unchanged. The standalone manifest resolves inherited
  package metadata and changes the vulnerable exact `h2 = "=0.4.13"` constraint
  to the patched compatible range `h2 = "0.4.16"`.
- The existing `impit` Git revision remains locked in the application lockfile.
  Original MIT licensing is retained and reproduced in `THIRD_PARTY_NOTICES`.

## Verification

The application's TLS regression tests cover malformed encryption-level
boundaries, large-window certificate compression, valid compression round trips,
and trailing compressed data. Provider conflict, feed transport, translation,
and embedded-production browser tests cover the integration boundary.

Review source provenance again whenever updating either vendored library. Audit
the application lockfile against current RustSec data as well as reviewing Git
and path dependencies, which the default registry audit does not cover.

## Reviewed residual audit findings

The 2026-10-28 RustSec snapshot
`40f5be6fb12ed93fc591bf4ab23b7d793b3f2239` reports one remaining registry
vulnerability, `RUSTSEC-2023-0071` (RSA Marvin), for the locked dependency graph.
SQLx MySQL is the only consumer of the `rsa` crate and uses `RsaPublicKey` to
encrypt an outgoing password with OAEP. It does not perform the private-key
operations affected by the timing attack. MySQL support is retained, the raw
finding is not hidden, and no persistent audit-ignore configuration is added.
Revisit this assessment if the RSA consumer graph or authentication code changes.

`encoding` and `proc-macro-error2` also have unmaintained-package notices; the
locked graph has no remaining unsoundness or yanked-package notices at that
snapshot. These notices are not described as repaired vulnerabilities.
