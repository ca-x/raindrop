# Article speech plugin

## Approved scope

The user requests an OpenAI-compatible TTS plugin, configuration and a real API
preview/test, manually selected voices, icon-only article controls, support in
both Web and the independent native desktop application, and new releases after
verification. Existing user work in `.nightshift/` is out of scope.

## Contract

- A bundled native `raindrop-tts` plugin owns validated speech parameters and
  Unicode-safe article chunking. The host owns storage, credentials, network
  policy and authentication. No change to the AI-content plugin ABI.
- Base URL is an API root, such as `https://example.com/v1`. The adapter appends
  `audio/speech`; an origin-only URL appends `v1/audio/speech`. HTTPS/public
  endpoints only, with existing pinned-DNS/peer-validation transport. No redirects.
- Per-user settings: enabled, base URL, model, voice, speed (0.25–4), revision.
  Voice is explicitly selected and persisted, with OpenAI and Edge examples plus
  arbitrary custom voice IDs. Never infer the voice from article language.
- API keys are optional for keyless compatible services; otherwise encrypted
  using the existing keyring and a TTS-specific encryption purpose. Keys never
  appear in GET responses, logs or playback URLs. Omitted key preserves it;
  null clears it. Revision comparisons prevent lost updates.
- `GET/PUT /api/v1/plugins/tts`: config envelope `{isEnabled,baseUrl,model,voice,
  speed,hasApiKey,keyringAvailable,revision}`. Initial revision is zero.
- `POST /api/v1/plugins/tts/test`: unsaved draft configuration plus `text`
  (max 200 characters), returns actual `audio/mpeg`. May use the stored key when
  apiKey is omitted. Explicit user action only; can incur provider charges.
- `POST /api/v1/plugins/tts/entries/{id}/plan`: returns
  `{entryId,contentHash,chunks:[string]}` from authorized article plain text.
- `POST /api/v1/plugins/tts/entries/{id}/speak` with
  `{contentHash,chunkIndex}`: verifies the current article/settings snapshot and
  returns one bounded MP3 chunk. The client cannot substitute arbitrary text.
- All mutations require authentication, CSRF and rate/concurrency budgets. No
  automatic retry of a synthesis POST. Generic redacted provider errors.

## Interaction

Article controls use accessible icons for read/pause/resume and stop. Loading,
disabled and error states are explicit; touch does not depend on hover. Sequential
chunk playback permits long articles without sending the whole document. Stop,
article changes, unmount and logout cancel pending work and release audio.
Settings provide labeled endpoint/key/model/voice/speed fields, a preview text,
real Test/Play action, persistence, and clear errors in Chinese and English.
Desktop remains native NSH and works through its in-process local core without
requiring a server; configured remote connections use the same authenticated API.

## Verification and delivery

Unit tests cover validation, exact OpenAI request fields, chunk boundaries and
snapshot checks. Backend tests cover config CAS, encryption/user isolation,
authentication/CSRF, fake upstream audio/error/oversize behavior and entry access.
Web/desktop tests cover selected voice persistence, preview and playback cleanup.
Run targeted tests, format/type/lint/build checks, then review the combined diff.
Release server and desktop on their separate version/tag lanes; do not build
Docker locally. Read back the remote release assets and checksums.
