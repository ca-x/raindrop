# Provider network access implementation

The user explicitly requested implementation and direct release. Spec: ../specs/2026-11-06-provider-network-access.md.

- [x] Accept HTTP/HTTPS and all literal/DNS address categories in the shared Provider boundary.
- [x] Remove the deployment allowlist and its propagation from configuration, discovery and generation.
- [x] Update the Provider form, generated API contracts, operator documentation and release versions.
- [x] Verify actual local HTTP requests across all four protocols and model discovery, plus the standard Rust/Web gates.
- [x] Review the complete diff and test the private HTTPS service with fresh credentials.
- [ ] Commit/push, verify CI, publish v0.6.6, inspect archives and image manifests, and smoke-test the published Linux binary.
- [ ] Stop isolated test processes and remove temporary credentials/session data.

## Verification

Strict MSRV 1.94 Clippy, formatting, rustdoc and workspace tests passed: 818 tests, zero failures, one opt-in public RSS test ignored. Web generated contracts, release contracts, typecheck, 567 tests in 79 files and production build passed. Actual local HTTP requests covered all four generation protocols and model discovery over IPv4, localhost and IPv6. The custom DeepLX connection/translation regression failed on the old HTTPS-only validator, then passed with saved credentials over localhost HTTP. Independent review found no release blockers.

A fresh private HTTPS credential returned valid model discovery, short summary, Chinese full and selection translations and two follow-up turns. An earlier model answer failed schema/finish validation and remained an explicit error; subsequent two-turn runs passed without loosening validation. The 32,195-character long-article test also passed: native translation used three requests, 18,300 output tokens and 52.45 seconds; reader translation took 85.83 seconds, and both retained all 153 source sections. Packaged-binary smoke and publication checks follow the release.
