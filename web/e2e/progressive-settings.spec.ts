import { expect, test } from "@playwright/test"

import type { AiConfigEnvelope, PutAiConfigRequest } from "../src/features/ai/api/content.generated"
import type { CreateProviderRequest, Provider } from "../src/features/ai/api/provider.generated"
import { completeSetup, createCredentials } from "./support/app"
import { installReaderApiFixture, readerIds } from "./support/readerApiFixture"
import { expectDialogContained, expectNoHorizontalOverflow } from "./support/readerAssertions"
import { startProductionServer, type ProductionServer } from "./support/productionServer"

let server: ProductionServer

test.beforeAll(async () => {
  const previewURL = process.env.RAINDROP_E2E_WEB_BASE_URL
  server = previewURL
    ? { baseURL: previewURL, setupToken: "unused", stop: async () => {} }
    : await startProductionServer()
})

test.afterAll(async () => { await server?.stop() })

test("progressive AI setup preserves drafts and stays usable on compact screens", async ({ page }, testInfo) => {
  const pageErrors: string[] = []
  page.on("pageerror", (error) => pageErrors.push(error.message))
  await installReaderApiFixture(page)
  const userId = "00000000-0000-4000-8000-000000000901"
  await page.route(/\/api\/v1\/subscriptions(?:\?.*)?$/u, (route) => route.fulfill({ json: { ownerUserId: userId, items: [], nextCursor: null } }))
  await page.route(/\/api\/v1\/categories(?:\?.*)?$/u, (route) => route.fulfill({ json: { ownerUserId: userId, items: [] } }))
  await page.route(/\/api\/v1\/entries(?:\?.*)?$/u, (route) => route.fulfill({ json: { ownerUserId: userId, items: [], nextCursor: null, snapshotGeneration: 0 } }))
  await page.route("**/api/v1/bootstrap", (route) => route.fulfill({ json: { status: "READY", version: "test-fixture" } }))
  await page.route("**/api/v1/auth/session", (route) => route.fulfill({ json: {
    user: { id: userId, username: "Reader", email: null, isDisabled: false, roles: ["ADMIN", "USER"] },
    csrfToken: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA", expiresAt: "2027-01-01T00:00:00Z",
  } }))
  await page.route("**/api/v2/profile", (route) => route.fulfill({ json: { userId, username: "Reader", displayName: null, email: null } }))
  await page.route("**/api/v3/plugins/translation", (route) => route.fulfill({ json: {
    engine: "DEEPLX", displayMode: "BILINGUAL", isEnabled: false, defaultTargetLocale: "zh-CN",
    openAi: { providerId: null, maxOutputTokens: 4096, profile: "GENERAL", customSystemPrompt: null, customPrompt: null },
    deepLx: { displayName: "DeepLX", description: null, baseUrl: null, isProgressive: true, hasApiKey: false }, revision: null,
  } }))
  await page.route("**/api/v1/events", (route) => route.fulfill({ contentType: "text/event-stream", body: "" }))
  await page.route("**/api/v1/ai/tambo", (route) => route.fulfill({ json: { configured: false, enabled: false, origin: null, revision: null } }))

  const providers: Provider[] = []
  await page.route("**/api/v1/ai/providers", async (route) => {
    if (route.request().method() === "GET") {
      await route.fulfill({ json: { keyringStatus: "AVAILABLE", items: providers } })
      return
    }
    const draft = route.request().postDataJSON() as CreateProviderRequest
    expect(route.request().headers()["x-csrf-token"]).toBeTruthy()
    const provider: Provider = {
      providerId: "00000000-0000-4000-8000-000000000902", scope: "USER", canEdit: true,
      displayName: draft.displayName, kind: draft.kind, endpoint: draft.endpoint ?? "https://api.openai.com/",
      model: draft.model, capabilities: draft.capabilities, policy: draft.policy, isEnabled: draft.isEnabled,
      revision: 0, createdAt: "2026-10-22T00:00:00Z", updatedAt: "2026-10-22T00:00:00Z",
    }
    providers.push(provider)
    await route.fulfill({ status: 201, json: provider })
  })
  await page.route("**/api/v1/ai/providers/models", (route) => route.fulfill({
    status: 504, json: { error: { code: "MODEL_DISCOVERY_TIMEOUT", message: "Lookup timed out" } },
  }))
  let envelope: AiConfigEnvelope = { pluginState: "READY", mcpState: "CONTRACT_READY_TRANSPORT_UNAVAILABLE", config: null }
  await page.route("**/api/v1/ai/config", async (route) => {
    if (route.request().method() === "PUT") {
      const draft = route.request().postDataJSON() as PutAiConfigRequest
      envelope = { ...envelope, config: {
        revision: (draft.expectedRevision ?? -1) + 1, isEnabled: draft.isEnabled,
        summary: draft.summary, translation: draft.translation,
      } }
    }
    await route.fulfill({ json: envelope })
  })

  await page.goto(`${server.baseURL}/reader/unread`, { waitUntil: "domcontentloaded" })
  await expect(page.getByRole("region", { name: "Entry queue" })).toBeVisible()
  const menu = page.getByRole("button", { name: "Open menu" })
  if (!(await menu.isVisible())) await page.getByRole("button", { name: "Open sources" }).click()
  await menu.click()
  await page.getByRole("menuitem", { name: "Settings", exact: true }).click()
  const dialog = page.getByRole("dialog", { name: "Settings" })
  await dialog.getByRole("button", { name: /^Plugins/u }).click()
  await expect(dialog.getByRole("heading", { name: "Make this reader your own" })).toBeVisible()
  await expectDialogContained(dialog, page)
  await page.screenshot({ path: testInfo.outputPath("plugins.png") })
  await dialog.getByRole("button", { name: "Connect a service" }).click()
  await dialog.getByRole("button", { name: "Add Provider" }).click()
  const form = dialog.locator(".ai-provider-form")
  await expect(form.getByRole("textbox", { name: /^Model/u })).toHaveCount(0)
  await form.getByRole("textbox", { name: /^Display name/u }).fill("My model service")
  await form.getByLabel(/^Credential/u).fill("e2e-placeholder-key")
  const nameBounds = await form.getByRole("textbox", { name: /^Display name/u }).boundingBox()
  const kindBounds = await form.locator(".astryx-selector").boundingBox()
  expect(nameBounds).not.toBeNull()
  expect(kindBounds).not.toBeNull()
  expect(Math.abs(nameBounds!.height - kindBounds!.height)).toBeLessThanOrEqual(1)
  await page.screenshot({ path: testInfo.outputPath("connection.png") })
  await form.getByRole("button", { name: "Continue to model" }).press("Enter")
  await expect(form).toHaveAttribute("data-input", "keyboard")
  await expect(form.getByRole("textbox", { name: /^Model/u })).toBeFocused()
  await expect(form.locator(".ai-provider-step-content")).toHaveCSS("transition-duration", "0s")
  await form.getByRole("textbox", { name: /^Model/u }).fill("model-manual")
  await form.getByRole("button", { name: "Back to connection" }).click()
  await expect(form.getByRole("textbox", { name: /^Display name/u })).toHaveValue("My model service")
  await expect(form.getByLabel(/^Credential/u)).toHaveValue("e2e-placeholder-key")
  await form.getByRole("button", { name: "Continue to model" }).click()
  await expect(form.getByRole("textbox", { name: /^Model/u })).toHaveValue("model-manual")
  await form.getByRole("button", { name: "Fetch models" }).click()
  await expect(form.getByRole("alert")).toContainText("The service took too long")
  await expect(form.getByRole("button", { name: "Create Provider" })).toBeEnabled()
  await expectNoHorizontalOverflow(page)
  await page.screenshot({ path: testInfo.outputPath("model-fallback.png") })
  await form.getByRole("button", { name: "Create Provider" }).click()
  await expect(dialog.getByText("Provider saved.", { exact: true })).toBeVisible()
  await dialog.getByRole("button", { name: "Continue to reading features" }).click()
  await dialog.getByRole("button", { name: "Next: set up summaries" }).click()
  await expect(dialog.getByRole("combobox", { name: "Summary Provider" })).toBeHidden()
  await dialog.getByRole("switch", { name: "Enable AI reading plugin" }).click()
  await expect(dialog.getByRole("switch", { name: "Enable summary" })).toBeChecked()
  await expect(dialog.getByRole("spinbutton")).toBeHidden()
  await dialog.getByRole("button", { name: "Output length and capabilities" }).click()
  await expect(dialog.getByRole("spinbutton")).toBeVisible()
  await page.emulateMedia({ reducedMotion: "reduce" })
  await dialog.getByRole("switch", { name: "Enable summary" }).click()
  await dialog.getByRole("switch", { name: "Enable summary" }).click()
  await expect(dialog.locator(".ai-content-form .ai-provider-step-content")).toHaveCSS("transform", "none")
  const save = dialog.getByRole("button", { name: "Save AI settings" })
  await save.scrollIntoViewIfNeeded()
  const beforeSave = await save.boundingBox()
  await save.click()
  await expect(dialog.getByRole("status").filter({ hasText: "AI reading settings saved." })).toBeVisible()
  const afterSave = await save.boundingBox()
  expect(Math.abs(afterSave!.y - beforeSave!.y)).toBeLessThanOrEqual(1)
  await expectDialogContained(dialog, page)
  await expectNoHorizontalOverflow(page)
  await page.screenshot({ path: testInfo.outputPath("summaries.png") })
  expect(pageErrors).toEqual([])
})

test("optional interaction stays lazy, streams through the local SDK endpoint, and restores focus", async ({ page }, testInfo) => {
  testInfo.setTimeout(60_000)
  const pageErrors: string[] = []
  const sdkLoads: string[] = []
  const requests: string[] = []
  page.on("pageerror", (error) => pageErrors.push(error.message))
  page.on("request", (request) => {
    if (/\/assets\/TamboConversation-/u.test(request.url())) sdkLoads.push(request.url())
  })
  await installReaderApiFixture(page)
  let preference = { configured: true, enabled: false, origin: "https://tambo.example", revision: null as number | null }
  await page.route("**/api/v1/ai/tambo", async (route) => {
    if (route.request().method() === "PUT") {
      expect(route.request().headers()["x-csrf-token"]).toBeTruthy()
      const body = route.request().postDataJSON() as { expectedRevision: number | null; enabled: boolean }
      expect(body.expectedRevision).toBe(preference.revision)
      preference = { ...preference, enabled: body.enabled, revision: (preference.revision ?? -1) + 1 }
    }
    await route.fulfill({ json: preference })
  })
  await page.route("**/api/v1/ai/tambo/api/**", async (route) => {
    const request = route.request()
    requests.push(request.url())
    if (request.method() === "POST") {
      expect(new URL(request.url()).pathname).toBe("/api/v1/ai/tambo/api/v1/threads/runs")
      expect(request.headers()["x-api-key"]).toBeTruthy()
      const body = request.postDataJSON() as { thread: { userKey: string }; message: { additionalContext: { article: { title: string; text: string } } } }
      expect(body.thread.userKey).toBe("authenticated-session")
      expect(body.message.additionalContext.article.title).toBe("First quiet article")
      expect(body.message.additionalContext.article.text.length).toBeLessThanOrEqual(12000)
      const events = [
        { type: "RUN_STARTED", threadId: "thread_1", runId: "run_1" },
        { type: "TEXT_MESSAGE_START", messageId: "answer_1", role: "assistant" },
        { type: "TEXT_MESSAGE_CONTENT", messageId: "answer_1", delta: "A streamed reading answer." },
        { type: "TEXT_MESSAGE_END", messageId: "answer_1" },
        { type: "RUN_FINISHED", threadId: "thread_1", runId: "run_1" },
      ]
      await route.fulfill({ contentType: "text/event-stream", body: events.map((event) => `data: ${JSON.stringify(event)}\n\n`).join("") })
    } else {
      await route.fulfill({ json: { threads: [], messages: [], hasMore: false } })
    }
  })
  await completeSetup(page, server, createCredentials())
  const menu = page.getByRole("button", { name: "Open menu" })
  if (!(await menu.isVisible())) await page.getByRole("button", { name: "Open sources" }).click()
  await menu.click()
  await page.getByRole("menuitem", { name: "Settings", exact: true }).click()
  const dialog = page.getByRole("dialog", { name: "Settings" })
  await dialog.getByRole("button", { name: /^Plugins/u }).click()
  await dialog.getByRole("button", { name: "Settings for AI Assistant", exact: true }).click()
  const enable = dialog.getByRole("switch", { name: "Enable interactive reading" })
  await enable.click()
  await expect(enable).toBeChecked()
  await expectDialogContained(dialog, page)
  await page.screenshot({ path: testInfo.outputPath("tambo-settings.png") })
  await page.goto(`${server.baseURL}/reader/unread/entry/${readerIds.firstEntry}`)
  const discuss = page.getByRole("button", { name: "Discuss article" })
  await discuss.click()
  const panel = page.getByRole("region", { name: "Interactive reading" })
  await expect(panel.getByText(/12,000 characters/u)).toContainText("https://tambo.example")
  expect(sdkLoads).toHaveLength(0)
  expect(requests).toHaveLength(0)
  await panel.getByRole("button", { name: "Start conversation" }).click()
  const composer = panel.getByRole("textbox")
  await expect(composer).toBeFocused()
  expect(sdkLoads).toHaveLength(1)
  await composer.fill("Summarize the key points")
  await panel.getByRole("button", { name: "Send", exact: true }).click()
  await expect(panel.getByText("A streamed reading answer.")).toBeVisible()
  const panelBounds = await panel.boundingBox()
  const sendBounds = await panel.getByRole("button", { name: "Send", exact: true }).boundingBox()
  expect(panelBounds).not.toBeNull()
  expect(sendBounds).not.toBeNull()
  expect(sendBounds!.height).toBeGreaterThanOrEqual(44)
  expect(sendBounds!.y + sendBounds!.height).toBeLessThanOrEqual(panelBounds!.y + panelBounds!.height)
  await page.emulateMedia({ reducedMotion: "reduce" })
  await expectNoHorizontalOverflow(page)
  await page.screenshot({ path: testInfo.outputPath("tambo-reading.png") })
  await panel.getByRole("textbox").focus()
  await page.keyboard.press("Escape")
  await expect(panel).toBeHidden()
  await expect(discuss).toBeFocused()
  expect(pageErrors).toEqual([])
})
