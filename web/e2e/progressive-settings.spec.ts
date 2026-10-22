import { expect, test } from "@playwright/test"

import type { AiConfigEnvelope, PutAiConfigRequest } from "../src/features/ai/api/content.generated"
import type { CreateProviderRequest, Provider } from "../src/features/ai/api/provider.generated"
import { installReaderApiFixture } from "./support/readerApiFixture"
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
