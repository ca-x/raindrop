# Long translation and summary questions

User requirements: translate long articles without a 4096-token article ceiling; size requests from content; add a composer beneath summaries for follow-up questions; use the existing component library and the animate, ui-ux-pro-max, emil-design-eng and apple-design skills; verify and publish.

## Capabilities and order

| Module | Responsibility | Dependency |
| --- | --- | --- |
| text-budget | Content-aware output estimates and lossless UTF-8 chunk boundaries | Existing Provider policy |
| long-translation | Translate every chunk and publish only the complete combined artifact; preserve reader segment order | text-budget |
| article-questions | Answer follow-up questions with the current article and bounded conversation context | Existing official native plugin and Provider broker |
| question-composer | Accessible input and conversation directly below a successful summary | article-questions |

Build in that order. Existing maxOutputTokens becomes a per-request ceiling, never an article-wide output ceiling. Provider input/output/cost/concurrency policy, deadlines, official signing, authentication, CSRF and ownership remain enforced. Do not silently truncate source or return successful partial translations. The user explicitly requested `models-dev-rs = "0.1"` for capacity metadata; no new database tables are required. Conversation state belongs to the current reading session and clears with the article/account; it is not represented as a saved chat history.

## Interaction

Keep Raindrop's existing reading typography, spacing and colors. Like Apple Notes, keep the input next to its content; like ChatGPT, keep a visible multiline composer and an explicit send/stop action; like Linear, show quiet inline pending/error feedback and retain failed input. Reuse ASTRYX TextArea, Button, Text, Heading and Banner. Provide named controls, visible focus, IME-safe keyboard behavior, 44px touch targets and mobile wrapping. Do not animate typing, scrolling or repeated keyboard actions. A short opacity transition may indicate a newly completed answer; respect reduced motion, contrast and transparency preferences. Never move text while it is being read or force-scroll a reader who is reviewing earlier answers.

## Acceptance and validation

- A long paragraph is split at UTF-8, paragraph, sentence or whitespace boundaries without losing source text; ordered outputs are merged back into the original reader segments.
- Native translation uses multiple separately validated Provider calls when needed; later failure produces no completed artifact. Total output can exceed 4096 tokens.
- Short and long inputs request different budgets; every individual call remains within the configured Provider limits.
- Summary questions use the configured summary Provider, article text and bounded history. User messages and article content cannot replace fixed system policy. Another user's article is unavailable and disabling AI prevents further questions.
- Composer handles first/repeated questions, waiting, stop, retry, article switch, unmount, authentication expiry, IME composition, narrow screens and reduced motion.
- Run Rust formatting, strict workspace Clippy, workspace tests and rustdoc; Web typecheck, Vitest, generated contracts, build and E2E. Exercise long translation and two-turn follow-up against the supplied test service. Publish only after final-source CI, then download and verify release packages and published images.

Commands: `cargo +1.94.0 fmt --all --check`; `cargo +1.94.0 clippy --locked --workspace --all-targets --all-features -- -D warnings`; `cargo +1.94.0 test --locked --workspace --all-features`; `cargo +1.94.0 doc --locked --workspace --no-deps`; `npm --prefix web run typecheck`; `npm --prefix web run test:ci`; `npm --prefix web run build`; `npm --prefix web run test:e2e`.

Rust 2024 / MSRV 1.94 and the repository's existing Result, typed errors, canonical schemas and host capability patterns apply. Keep generated API contracts synchronized. Never put test credentials, service addresses or private reports in release notes or committed fixtures.

## Model capacity and configurable long-text budgets

The later user requirement adds `models-dev-rs = "0.1"` for typed, optional capacity hints. Fetch only the fixed public models.dev catalog, without any Provider credentials. Cache for six hours, back off failed refreshes for ten minutes, keep the last good catalog, and preserve manual setup when unavailable. Prefer exact provider-qualified IDs, then normalize gateway namespaces and separators (dots, hyphens, underscores, colons), preserving digits/sizes/variant words. Return unknown on conflicting catalog aliases and label alias matches in the UI; known smaller service limits survive whitespace and aliases. Provider-discovered smaller limits win. Catalog metadata cannot change routing, permissions or existing saved limits.

Raise the application validation ceiling for per-request output to 1,048,576 tokens. New Provider and translation defaults are 32,768; new summary defaults are 8,192. Existing values remain unchanged. Content-based estimates continue below these configurable ceilings, and complete article output can exceed a single request. The capacity action fills the draft only after the user clicks, reserves input space in the shared context window, and leaves the settings editable.
