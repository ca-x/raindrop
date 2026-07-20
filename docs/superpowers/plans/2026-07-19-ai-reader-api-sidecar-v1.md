# AI Reader API and Sidecar v1 Implementation Plan

> **Execution:** Inline main-agent execution only. The user explicitly prohibited sub-agent development. Steps use checkbox syntax for tracking.

**Goal:** Connect the existing Provider, official AI plugin, content jobs, artifacts, and production worker to a user-scoped settings flow and a non-blocking Reader summary/translation sidecar.

**Architecture:** Parse one optional Provider keyring in main and share it with HTTP and background runtime composition. Build current artifact identities in a focused content service, expose strict Provider/config/job APIs from committed OpenAPI, then consume those contracts in feature-modular ASTRYX settings and Reader components.

**Tech Stack:** Rust 2024/MSRV 1.94, Axum, SeaORM, Tokio, Wasmtime Component Model, React 19, TypeScript 7, Lingui, ASTRYX 0.1.6, Vitest, Playwright.

## Global constraints

- Main agent only, one bounded DDIA/API/security/UI self-review.
- Work directly on main and preserve unrelated user changes.
- Original article is the default and never waits on AI.
- No automatic lifecycle enqueue, MCP transport, Provider physical delete, streaming, chat, or third-party plugin UI in this slice.
- Credential, encrypted envelope, raw payload JSON, and raw provenance JSON never enter public responses or frontend state.
- No new Rust crate or npm package.
- Exact apply_patch edits and exact staging, never git add -A.
- Rust commands use env -u RUSTUP_TOOLCHAIN cargo +1.94.0.

---

### Task 1: Share an optional Provider keyring across HTTP and worker composition

**Files:**

- Modify: src/content/provider/repository.rs
- Modify: src/content/worker/production.rs
- Modify: src/background.rs
- Modify: src/app.rs
- Modify: src/main.rs
- Test: tests/ai_provider_storage.rs
- Test: tests/content_runtime_production.rs

**Interfaces:**

- ProviderRepository::new(database, Option<Arc<ProviderSecretKeyring>>)
- AppState::with_runtime_services(setup, feed_handle, content_handle, provider_keyring)
- BackgroundRuntime::production(setup, retention, provider_keyring)
- ProductionContentRuntime::new(setup, provider_keyring)

- [x] **Step 1: Add failing metadata-only Provider repository tests**

Cover list/get and non-credential update with None keyring, plus create and credential rotation returning SecretUnavailable.

Run:

~~~bash
env -u RUSTUP_TOOLCHAIN cargo +1.94.0 test --locked --all-features --test ai_provider_storage metadata_only -- --nocapture
~~~

Expected: compile failure because ProviderRepository still requires an owned keyring.

- [x] **Step 2: Change ProviderRepository to an optional Arc keyring**

Use the exact field:

~~~rust
keyring: Option<Arc<ProviderSecretKeyring>>
~~~

Create and credential rotation obtain keyring through a private require_keyring helper. List/get and patch without credential do not require it. load_enabled_binding always requires it. Preserve redacted error mapping.

- [x] **Step 3: Add failing production composition tests**

Prove one Arc keyring can be cloned into AppState and ProductionContentRuntime, and None keeps runtime inert without preventing metadata access.

Run:

~~~bash
env -u RUSTUP_TOOLCHAIN cargo +1.94.0 test --locked --all-features --test content_runtime_production -- --test-threads=1
~~~

Expected: compile failure until constructors accept Option<Arc<ProviderSecretKeyring>>.

- [x] **Step 4: Parse the keyring once in main**

Convert loaded entries immediately:

~~~rust
let provider_keyring = if provider_secret_keys.is_empty() {
    None
} else {
    Some(Arc::new(ProviderSecretKeyring::from_entries(&provider_secret_keys)?))
};
~~~

Do not retain provider_secret_keys after conversion. Pass clones to background and AppState. Add provider_mutation_limiter and content_mutation_limiter to AppState and preserve test constructors with None.

- [x] **Step 5: Run focused composition regressions**

~~~bash
env -u RUSTUP_TOOLCHAIN cargo +1.94.0 test --locked --all-features --test ai_provider_storage
env -u RUSTUP_TOOLCHAIN cargo +1.94.0 test --locked --all-features --test content_runtime_production -- --test-threads=1
env -u RUSTUP_TOOLCHAIN cargo +1.94.0 test --locked --all-features --lib app::tests background::tests
~~~

Expected: all pass and secret-bearing Debug assertions remain redacted.

### Task 2: Add user-scoped Provider API and OpenAPI contract

**Files:**

- Create: src/api/ai/mod.rs
- Create: src/api/ai/providers.rs
- Modify: src/api/mod.rs
- Modify: src/api/routes.rs
- Modify: src/api/error.rs
- Modify: src/content/provider/repository.rs
- Create: docs/openapi/ai-provider-v1.json
- Create: tests/ai_provider_api.rs
- Create: tests/ai_provider_openapi_contract.rs

**Interfaces:**

- GET/POST /api/v1/ai/providers
- GET/PATCH /api/v1/ai/providers/:providerId
- ProviderRepository::get_visible_for_user
- ProviderRepository::count_user_owned

- [x] **Step 1: Write the Provider HTTP tracer tests**

Create a ready SQLite fixture with two users, one instance Provider, one Provider per user, session cookies, CSRF, and a configured keyring. First tests assert:

- user list includes instance plus own Provider only;
- response has keyringStatus and no credential/encryptedSecret/key id;
- create returns 201 and Location;
- detail returns the same safe item shape.

Run:

~~~bash
env -u RUSTUP_TOOLCHAIN cargo +1.94.0 test --locked --all-features --test ai_provider_api provider_tracer -- --nocapture
~~~

Expected: FAIL with route not found.

- [x] **Step 2: Implement strict Provider request and response DTOs**

Use deny_unknown_fields, camelCase fields, exact UPPER_SNAKE enums, nullable numeric policy fields, and SecretString conversion at the handler boundary. Response mapping includes scope/canEdit and never serializes credential data.

POST builds:

~~~rust
CreateProvider {
    scope: ProviderScope::user(user.id.clone())?,
    display_name,
    kind,
    endpoint,
    model,
    credential: SecretString::from(credential),
    capabilities,
    policy,
    is_enabled,
}
~~~

- [x] **Step 3: Implement visibility, revision, keyring, and limit behavior**

Add get_visible_for_user without decrypting. PATCH always uses ProviderScope::user, so instance and cross-user IDs return 404. count_user_owned enforces 32 before create. Map SecretUnavailable to 503 AI_PROVIDER_KEYRING_UNAVAILABLE, revision conflict to 409, invalid fields to 422, and count limit to 409 PROVIDER_LIMIT_REACHED.

- [x] **Step 4: Add auth, CSRF, rate, path, method, and cache tests**

Cover missing session, missing CSRF, malformed UUID, unknown field, empty patch, null/omitted credential, metadata-only patch without keyring, credential rotation without keyring, per-user rate isolation, trailing slash, unknown child path, 405, and no-store on success/failure.

- [x] **Step 5: Commit the Provider OpenAPI artifact and drift gate**

The artifact defines all request/response schemas, security, Location, Retry-After, no-store headers, and stable errors. The real-router contract test validates representative 200/201/401/403/404/409/422/429/503/500 responses.

Run:

~~~bash
env -u RUSTUP_TOOLCHAIN cargo +1.94.0 test --locked --all-features --test ai_provider_api --test ai_provider_openapi_contract
git diff --check
~~~

Expected: all pass.

### Task 3: Expose typed official AI configuration

**Files:**

- Modify: src/plugins/config.rs
- Modify: src/plugins/mod.rs
- Create: src/api/ai/config.rs
- Modify: src/api/ai/mod.rs
- Create: tests/ai_config_api.rs
- Modify: docs/openapi/ai-content-v1.json

**Interfaces:**

- public AiSummaryStyle enum and getters for stored config
- GET/PUT /api/v1/ai/config
- canonical API config builder with fixed disabled MCP and automatic blocks

- [x] **Step 1: Add failing public config-view tests**

Test SummaryStyle values, summary provider/enabled/style/token getters, translation provider/enabled/locale/token getters, and canonical round-trip. Keep MCP details private except boolean disabled getters already used by worker.

Run:

~~~bash
env -u RUSTUP_TOOLCHAIN cargo +1.94.0 test --locked --all-features plugins::config::tests
~~~

Expected: compile failure for missing public style/view API.

- [x] **Step 2: Add GET config tests**

Assert config null for an active user without a row, READY installation state for the bundled plugin, exact typed response for a stored config, and no raw canonical JSON/config hash in the wire response.

- [x] **Step 3: Add PUT config tests**

Cover create with expectedRevision null, replace with exact revision, stale revision 409, isEnabled equivalence, no operations enabled, missing/disabled/cross-user Provider, instance Provider selection, unknown fields, invalid locale/token bounds, CSRF, rate limit, no-store, and plugin disabled/quarantined.

- [x] **Step 4: Implement the config API**

Build the canonical config with serde_json::json and AiContentConfig::parse before PluginRegistryRepository::replace_ai_config. Fixed subtrees:

~~~json
{
  "mcp": {
    "mode": "DISABLED",
    "failurePolicy": "FAIL_OPEN",
    "maxToolCalls": 0,
    "tools": []
  },
  "automatic": {
    "enabled": false,
    "operations": ["SUMMARIZE", "TRANSLATE"],
    "allSubscribedFeeds": false,
    "feedIds": [],
    "categoryIds": []
  }
}
~~~

Validate selected Provider through get_visible_for_user and isEnabled. Return pluginState and mcpState independently from config.

- [x] **Step 5: Extend ai-content OpenAPI and run focused tests**

~~~bash
env -u RUSTUP_TOOLCHAIN cargo +1.94.0 test --locked --all-features --test ai_config_api
env -u RUSTUP_TOOLCHAIN cargo +1.94.0 test --locked --all-features --test plugin_registry_repository
git diff --check
~~~

Expected: all pass.

### Task 4: Build current AI identity and enqueue/retry service

**Files:**

- Create: src/content/ai/mod.rs
- Create: src/content/ai/service.rs
- Modify: src/content/mod.rs
- Create: src/content/worker/contracts.rs
- Modify: src/content/worker/mod.rs
- Modify: src/content/worker/processor.rs
- Modify: src/content/jobs/repository.rs
- Create: tests/ai_content_service.rs

**Interfaces:**

- official_ai_contract(ContentJobOperation)
- ContentRepository::get_execution_entry_for_user
- ContentRepository::find_latest_job_by_identity
- AiContentService::overview
- AiContentService::enqueue
- AiContentService::retry

- [x] **Step 1: Add a failing shared-contract test**

Assert summary/translation prompt versions and schema IDs match the processor and official artifact schemas. Move constants into contracts.rs and make both processor and service consume the function.

- [x] **Step 2: Add execution projection tests**

get_execution_entry_for_user must:

- require a visible subscribed entry;
- return feed id, content hash, title, rendered text, canonical URL;
- preserve the 512 KiB text cap;
- return NotFound for cross-user/invisible IDs;
- keep contentHash out of public Reader DTOs.

- [x] **Step 3: Add latest identity job tests**

Insert two jobs with the same identity and different creation ordering, plus a hash-collision fixture with mismatching stored identity fields. Return the newest verified job or corruption.

- [x] **Step 4: Implement identity construction**

AiContentService loads current entry, installation, config, operation settings, and visible Provider metadata. It creates ContentInvocationInput and:

~~~rust
ArtifactIdentity::new(ArtifactIdentityInput {
    user_id,
    entry_id,
    kind,
    target_locale,
    entry_content_hash,
    input_hash,
    config_hash,
    plugin_key,
    plugin_version,
    component_digest,
    provider_binding_id,
    provider_kind,
    provider_model,
    provider_revision,
    prompt_version,
    schema_id,
    mcp_provenance_hash: disabled_mcp_provenance_hash(),
})
~~~

- [x] **Step 5: Implement enqueue semantics**

Validate operation/locale/idempotency key. Check for current artifact before requiring keyring. If no artifact and keyring is absent, return KeyringUnavailable without creating a job. Otherwise call ContentRepository::enqueue and notify ContentRuntimeHandle only after success.

- [x] **Step 6: Implement current-snapshot manual retry**

Load the old job by current user, require FAILED, read its operation and target locale, then call the same enqueue path with the new idempotency key. Never edit the old job or reuse its identity.

- [x] **Step 7: Run service and worker regressions**

~~~bash
env -u RUSTUP_TOOLCHAIN cargo +1.94.0 test --locked --all-features --test ai_content_service
env -u RUSTUP_TOOLCHAIN cargo +1.94.0 test --locked --all-features --test content_job_enqueue --test content_job_claims --test content_job_terminals
env -u RUSTUP_TOOLCHAIN cargo +1.94.0 test --locked --all-features --test content_worker_processor
~~~

Expected: all pass.

### Task 5: Add entry AI overview, job, result, and retry APIs

**Files:**

- Create: src/api/ai/content.rs
- Modify: src/api/ai/mod.rs
- Modify: src/api/error.rs
- Modify: docs/openapi/ai-content-v1.json
- Create: tests/ai_content_api.rs
- Create: tests/ai_content_openapi_contract.rs
- Modify: web/scripts/generate-reader-types.mjs

**Interfaces:**

- GET /api/v1/entries/:entryId/ai
- POST /api/v1/entries/:entryId/ai/jobs
- GET /api/v1/ai/jobs/:jobId
- GET /api/v1/ai/jobs/:jobId/result
- POST /api/v1/ai/jobs/:jobId/retry
- generated provider.generated.ts and content.generated.ts

- [x] **Step 1: Write overview and enqueue tracer tests**

Cover NOT_CONFIGURED, DISABLED, PROVIDER_UNAVAILABLE, IDLE, QUEUED, RETRY_WAIT, SUCCEEDED, FAILED, translation locale override, reused artifact, 201/200 disposition, Location, runtime notification, and missing-keyring 503 without a database job.

- [x] **Step 2: Write status/result/retry tracer tests**

Status exposes only safe job fields. Result reparses payload through SummaryArtifact or TranslationArtifact and returns typed fields. Result before success is 409. Retry creates a different job ID and leaves old row/attempts unchanged.

- [x] **Step 3: Implement response mapping**

Use a tagged artifact enum:

~~~rust
enum AiArtifactResponse {
    Summary(SummaryArtifactResponse),
    Translation(TranslationArtifactResponse),
}
~~~

Do not expose payload_json, provenance_json, config hash, identity hash, provider endpoint, or credential state. User-facing lastErrorCode stays the existing stable worker code.

- [x] **Step 4: Complete error, path, method, cache, and tenant tests**

Cover invalid UUID/locale/idempotency, missing auth/CSRF, cross-user entry/job/artifact, retry non-failed, unknown paths, 405, rate limit, and no-store for every branch.

- [x] **Step 5: Finalize ai-content OpenAPI and generated clients**

Register both new artifacts in generate-reader-types.mjs:

~~~javascript
{
  source: "docs/openapi/ai-provider-v1.json",
  output: "src/features/ai/api/provider.generated.ts",
  aliases: {},
},
{
  source: "docs/openapi/ai-content-v1.json",
  output: "src/features/ai/api/content.generated.ts",
  aliases: {},
},
~~~

Run:

~~~bash
npm --prefix web run generate:reader-types
npm --prefix web run check:reader-types
env -u RUSTUP_TOOLCHAIN cargo +1.94.0 test --locked --all-features --test ai_content_api --test ai_content_openapi_contract
~~~

Expected: generated files are stable and all contract tests pass.

- [x] **Step 6: Commit and push the backend vertical slice**

Stage only backend, OpenAPI, generated DTO, tests, spec, plan, and task files. Inspect staged secret patterns and diff, then commit:

~~~bash
git commit -m "feat: expose AI reader services"
git push origin main
~~~

Monitor the resulting CI while continuing frontend work only if the local focused gates are green.

### Task 6: Build feature-modular ASTRYX AI settings

**Files:**

- Create: web/src/features/ai/api/providers.ts
- Create: web/src/features/ai/api/content.ts
- Create: web/src/features/ai/model/providerDraft.ts
- Create: web/src/features/ai/model/useAiSettingsController.ts
- Create: web/src/features/ai/settings/AiSettingsPanel.tsx
- Create: web/src/features/ai/settings/ProviderList.tsx
- Create: web/src/features/ai/settings/ProviderForm.tsx
- Create: web/src/features/ai/settings/AiContentForm.tsx
- Create: web/src/features/preferences/components/AppearancePreferencesForm.tsx
- Modify: web/src/features/preferences/components/PreferencesDialog.tsx
- Modify: web/src/features/reader/ReadyPage.tsx
- Test: adjacent API/model/component tests

**Interfaces:**

- AiSettingsController load/saveProvider/saveConfig/cancel
- PreferencesDialog optional initialTab and AI panel controller props
- no credential value in Provider response or persisted frontend cache

- [x] **Step 1: Write generated-boundary API tests**

Validate list/create/get/patch/config requests, CSRF headers, AbortSignal, 201/200 handling, generated response validators, and invalid response rejection. Assert request errors never include the credential string.

- [x] **Step 2: Write provider draft tests**

Define kind defaults for endpoint, capabilities, and bounded policy. Validate display name, model, required credential on create, optional credential on edit, numeric ranges, and unchanged-secret semantics.

- [x] **Step 3: Write AI settings controller tests**

Cover parallel Provider/config load, abort on logout/unmount, provider revision replacement, conflict preservation, config create/update, plugin/keyring unavailable state, and no mutation of the appearance controller.

- [x] **Step 4: Split the existing appearance form**

Move only the current appearance controls into AppearancePreferencesForm. PreferencesDialog retains one ASTRYX Dialog and introduces:

~~~tsx
<TabList value={tab} onChange={setTab} layout="fill" hasDivider>
  <Tab value="appearance" label={i18n._("preferences.appearanceTab")} />
  <Tab value="ai" label={i18n._("ai.settingsTab")} />
</TabList>
~~~

No nested dialog and no local generic control wrapper.

- [x] **Step 5: Implement Provider list and inline editor**

Use List/Item for providers, Badge or StatusDot for state, Button for Edit/Add, and one Collapsible for the editor. Base fields use TextInput/Selector/CheckboxInput. Advanced policy uses NumberInput inside a second Collapsible. Save and Cancel are explicit.

- [x] **Step 6: Implement AI content form**

Use CheckboxInput for enabled operations, Selector for Provider and target locale, SegmentedControl for summary style, NumberInput for token limits, Banner for MCP unavailable, and one Save AI settings action.

- [x] **Step 7: Add zh-CN and en messages**

Add concise copy for Provider kinds, secret retention, keyring/plugin state, summary style, target locale, errors, revision conflict, MCP contract status, and save actions. Avoid marketing language and exclamation marks.

- [x] **Step 8: Run settings tests**

~~~bash
npm --prefix web run typecheck
npm --prefix web run test:ci -- --run web/src/features/ai web/src/features/preferences/components/PreferencesDialog.test.tsx
~~~

Expected: focused tests and typecheck pass.

### Task 7: Add the non-blocking Reader AI sidecar

**Files:**

- Create: web/src/features/ai/model/useEntryAiController.ts
- Create: web/src/features/ai/reader/AiReaderSidecar.tsx
- Create: web/src/features/ai/reader/AiOperationState.tsx
- Create: web/src/features/ai/reader/SummaryView.tsx
- Create: web/src/features/ai/reader/TranslationView.tsx
- Modify: web/src/features/reader/components/ReaderToolbar.tsx
- Modify: web/src/features/reader/components/ArticleReader.tsx
- Modify: web/src/features/reader/reader.css
- Modify: web/src/features/reader/model/controllerApi.ts
- Test: web/src/features/reader/ReaderArticleWorkspace.test.tsx
- Test: adjacent AI controller/component tests

**Interfaces:**

- useEntryAiController(entryId, csrfToken, onUnauthenticated)
- ArticleToolbar onOpenSummary/onOpenTranslation
- AiReaderSidecar openTab, close, enqueue, retry, polling

- [x] **Step 1: Write controller race and polling tests**

Cover lazy overview load, one request per selected entry, abort on entry change, late-response suppression, polling only for QUEUED/RUNNING/RETRY_WAIT, result fetch on success, stop on close/unmount, unauthenticated propagation, and retry with a new idempotency key.

- [x] **Step 2: Write original-content persistence tests**

Render an article, set article scrollTop, open and switch sidecar tabs, complete a summary, close sidecar, and assert the same article DOM node, title focus target, content HTML, read/star state, and scroll offset remain.

- [x] **Step 3: Implement toolbar actions and lazy sidecar**

Summary and Translate are secondary ASTRYX buttons or labeled IconButtons with 44px hit targets. Sidecar is closed by default and mounted between toolbar and article scroll plane without creating a fourth Reader column.

- [x] **Step 4: Implement operation states**

- UNAVAILABLE/DISABLED: Banner and Open AI settings action.
- IDLE: explicit Run action.
- QUEUED/RUNNING/RETRY_WAIT: Spinner or ProgressBar with no fake percentage.
- FAILED: stable error copy and Retry.
- SUCCEEDED: typed view.

Use aria-live polite for status and preserve trigger focus on close.

- [x] **Step 5: Render validated artifacts**

Summary uses Heading, Text, and List. Translation uses:

~~~tsx
<Markdown
  headingLevelStart={3}
  contentWidth="100%"
  onLinkClick={safeExternalMarkdownLink}
>
  {artifact.bodyMarkdown}
</Markdown>
~~~

safeExternalMarkdownLink permits only http/https and returns false otherwise. Do not use dangerouslySetInnerHTML for AI output.

- [x] **Step 6: Implement responsive geometry**

Desktop max sidecar height is 320px. Compact max height is 45dvh. Use one named sidecar gap/padding token, independent overflow-y auto, overscroll containment, safe-area-aware compact padding, and 44px targets. Reduced motion removes spatial transition.

- [x] **Step 7: Run Reader and web regressions**

~~~bash
npm --prefix web run typecheck
npm --prefix web run test:ci
npm --prefix web run build
~~~

Expected: all pass with no generated type drift.

### Task 8: Verify real production UI, document, commit, push, and monitor CI

**Files:**

- Modify: README.md
- Modify: docs/configuration.md
- Modify: docs/ai-providers.md
- Modify: web/DESIGN.md only if the implemented sidecar changes an existing documented rule
- Modify: tasks/plan.md
- Modify: tasks/todo.md
- Modify: this plan

- [x] **Step 1: Run focused Rust API/service gates**

~~~bash
env -u RUSTUP_TOOLCHAIN cargo +1.94.0 fmt --all -- --check
env -u RUSTUP_TOOLCHAIN cargo +1.94.0 test --locked --all-features --test ai_provider_api --test ai_provider_openapi_contract
env -u RUSTUP_TOOLCHAIN cargo +1.94.0 test --locked --all-features --test ai_config_api
env -u RUSTUP_TOOLCHAIN cargo +1.94.0 test --locked --all-features --test ai_content_service --test ai_content_api --test ai_content_openapi_contract
env -u RUSTUP_TOOLCHAIN cargo +1.94.0 test --locked --all-features --test content_worker_processor --test content_runtime_production -- --test-threads=1
~~~

- [x] **Step 2: Run full web and Rust gates**

~~~bash
npm --prefix web run check:reader-types
npm --prefix web run typecheck
npm --prefix web run test:ci
npm --prefix web run build
env -u RUSTUP_TOOLCHAIN cargo +1.94.0 clippy --locked --all-targets --all-features -- -D warnings
env -u RUSTUP_TOOLCHAIN cargo +1.94.0 test --locked --all-features
git diff --check
~~~

- [x] **Step 3: Verify with local agent-browser**

Run a production binary with a temporary SQLite data directory and configured test keyring. Use a deterministic local provider fixture through the existing test transport seam, import or subscribe to https://www.ithome.com/rss/ if network is available, then verify:

- create Provider and config;
- open an actual entry;
- summary and translation terminal states;
- original article remains visible and scroll position survives;
- settings and sidecar at 1280x800, 900x800, 390x844, 360x800;
- zh-CN and en;
- light and dark;
- no page errors, console errors, horizontal overflow, clipped controls, or credential text.

If live network is unavailable, keep the real RSS smoke separate and use the committed deterministic Reader fixture for the release gate.

- [x] **Step 4: Update documentation and task truth**

Document keyring requirement, four Provider kinds, endpoint safety, no credential readback, explicit manual execution, internal/manual retry distinction, current artifact identity, MCP unavailable state, and original-content guarantee. Mark only implemented checklist items complete.

- [x] **Step 5: Perform one bounded final self-review**

Review only confirmed Critical/Important findings across DDIA, API, security, UI, and scope. Fix once inline. Do not start a repeated review loop.

- [x] **Step 6: Stage exact frontend/doc files and inspect**

Run a staged diff secret scan for credential/token/key material, verify generated files match committed OpenAPI, and inspect git diff --cached --stat plus git diff --cached.

- [ ] **Step 7: Commit and push the frontend vertical slice**

~~~bash
git commit -m "feat: add AI reader sidecar"
git push origin main
~~~

- [ ] **Step 8: Monitor CI to terminal success**

Confirm supply-chain audit, ASTRYX web, Rust foundation, current-stable compatibility, release E2E, and non-root container health. Fix only real failures, push, and monitor the replacement run.

## Self-review

- Spec coverage: shared keyring, Provider API, config API, current identity, enqueue/status/result/retry, settings, sidecar, responsive behavior, security, OpenAPI, docs, push, and CI each map to a task.
- Placeholder scan: every task has exact files, interfaces, commands, expected behavior, and no TBD/implement-later instruction.
- Type consistency: the Provider keyring type is shared end to end; service methods consume the same operation contract as processor; OpenAPI outputs map to the named frontend files; Settings and Reader controllers consume generated DTOs.
- DDIA: no external execution enters a transaction; artifact reuse is full-identity only; manual retry creates new history; Provider delete stays out until referential semantics exist.
- Security: credential confinement, tenant 404, CSRF, dedicated limits, safe endpoint, safe Markdown, untrusted model output, cost bounds, and no-store each have a direct test.
- UI: ASTRYX first, no nested settings dialog, no fourth Reader column, original article default, no fake MCP or fake progress, mobile and locale browser checks.
