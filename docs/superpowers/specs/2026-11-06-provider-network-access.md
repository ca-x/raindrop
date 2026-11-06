# Provider network access

## Goal

Users can connect an AI Provider hosted on any network reachable from the Raindrop server, without deploying a separate allowlist. This requirement supersedes the public-only/HTTPS-only rules and the temporary trusted-endpoints exception in earlier Provider specifications.

## Behavior

- Accept HTTP and HTTPS base URLs with internal hostnames, localhost, IPv4 and IPv6, including private and loopback addresses.
- Apply the same endpoint semantics to model discovery, native content jobs, reader translation and summary questions.
- Preserve endpoint path-prefix normalization and reject credentials embedded in URLs, query strings, fragments and ambiguous paths.
- Preserve HTTPS certificate validation, bounded/pinned DNS resolution, connected-peer checks, no redirects/proxy, timeouts and response bounds.
- Stop reading the old trusted-endpoints environment variable and TOML list. Existing TOML remains loadable; saved Providers and encrypted credentials need no migration.
- Custom DeepLX translation shares the Provider transport; accept HTTP/HTTPS LAN endpoints for its connection test and translation as well.
- Keep Feed and backup networking rules unchanged.

## Acceptance

Exercise actual HTTP requests for all four protocols and model discovery over loopback IPv4, localhost and loopback IPv6, including prefixed routes and credential headers. Verify private/mixed DNS answers remain pinned and malformed/oversized answers still fail. Run the standard Rust/Web checks. Verify a real private-network HTTPS service with the supplied test credential, then publish v0.6.6 and inspect its delivered artifacts. Credentials and private service details stay outside the repository and public release notes.
