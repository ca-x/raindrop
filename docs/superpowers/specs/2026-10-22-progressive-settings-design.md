# Progressive settings and quiet updates

## Scope

Refine the existing Raindrop reading workspace without replacing ASTRYX or its warm paper-and-ink visual language. Release as v0.4.22, maintaining existing API contracts and saved provider compatibility.

## Experience

- The plugin index explains the sequence: connect a provider, then optionally enable summaries or translation. Offer a direct next action without hiding independent translation setup.
- New providers use two steps: connection details, then model selection and optional limits. Existing providers remain directly editable. Allow returning to connection details without losing input.
- Model discovery is optional, cancellable, bounded, and never represented as proof that generation works. Manual model entry remains available when discovery fails or is unsupported.
- Show specific timeout, authentication, rate-limit, and network feedback next to discovery. Keep drafts after failure. Clear stale discovery results when connection settings change.
- Summary options appear after enabling the assistant. Keep advanced token limits under disclosure, and expose actual summary enablement rather than silently setting it.
- RSS refreshes remain in the background. Use a compact status/disclosure and an explicit new-article action. Never merge new arrivals or change reading selection automatically.

## Reliability

- Accept provider roots and versioned API bases without duplicating `/v1` or `/v1beta`; preserve gateway prefixes and strict HTTPS/public-network validation.
- Bound model discovery DNS, connection, total request time, and streamed response bytes. Retain DNS pinning, disabled redirects/proxies/compression, authorization, CSRF, and stored-credential isolation.
- Do not add automatic generation retries or unsafe protocol fallback. Do not claim model catalogs prove structured output support.

## Motion and accessibility

- Preserve system typography and current tokens. No new animation library, glass effects, or moving article rows.
- Occasional pointer-driven step changes use opacity and transform, existing `--ease-out`, and 180ms timing. Keyboard actions stay instant. Motion is interruptible and never blocks input.
- Reduced motion uses opacity only. Use visible focus, semantic progress, polite status updates, and narrow-screen layouts at 390px and 360px.

## Acceptance

Regression coverage for forward/backward setup, validation, manual discovery fallback, cancellation/stale requests, summary toggles, endpoint joining, and bounded discovery responses. Run frontend tests/typecheck/build, Rust formatting/Clippy/tests, generated/release contract checks, and browser checks. Synchronize release versions, commit main, create an annotated tag, push atomically, and read back remote refs and workflow status.
