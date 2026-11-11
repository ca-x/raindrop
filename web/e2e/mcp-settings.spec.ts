import { mkdir } from "node:fs/promises"
import { fileURLToPath } from "node:url"
import { expect, test, type APIRequestContext, type Locator, type Page } from "@playwright/test"

import type { CreatedMcpToken, McpToken } from "../src/features/mcp/api"
import { completeSetup, createCredentials } from "./support/app"
import { installReaderApiFixture, readerIds } from "./support/readerApiFixture"
import { expectDialogContained, expectNoHorizontalOverflow } from "./support/readerAssertions"
import { startProductionServer, type ProductionServer } from "./support/productionServer"

const protocolVersion = "2025-11-25"
let server: ProductionServer

test.use({ screenshot: "off", trace: "off", video: "off" })
test.beforeEach(async () => { server = await startProductionServer() })
test.afterEach(async () => { await server?.stop() })

test("MCP settings create a one-time read-only credential and revoke real transport access", async ({ page, playwright }, testInfo) => {
  testInfo.setTimeout(90_000)
  const pageErrors: string[] = []
  page.on("pageerror", (error) => pageErrors.push(error.name))
  await page.emulateMedia({ colorScheme: "light", reducedMotion: "reduce" })
  await completeSetup(page, server, createCredentials())
  const settings = await openMcpSettings(page)
  await expect(settings.getByText("Read-only access", { exact: true })).toBeVisible()
  await expect(settings.getByText(/No tokens yet\./u)).toBeVisible()
  await expectDialogContained(settings, page)
  await expectMcpContentContained(settings)
  await expectNoHorizontalOverflow(page)

  const tokenName = "Personal AI client"
  await settings.getByRole("textbox", { name: /^Token name/u }).fill(tokenName)
  await settings.getByRole("combobox", { name: "Expires after" }).click()
  await page.getByRole("option", { name: "30 days", exact: true }).click()
  const creation = page.waitForResponse((response) =>
    new URL(response.url()).pathname === "/api/v1/mcp/tokens" && response.request().method() === "POST",
  )
  await settings.getByRole("button", { name: "Create token", exact: true }).click()
  const creationResponse = await creation
  expect(creationResponse.status()).toBe(200)
  const created: CreatedMcpToken = await creationResponse.json()
  const secretField = settings.getByRole("textbox", { name: "Access token", exact: true })
  await expect(secretField).toBeVisible()
  await expect(secretField).toBeFocused()
  expect(created.token.length > 30 && await secretField.inputValue() === created.token).toBe(true)
  expect(created.item.name).toBe(tokenName)
  expect(Date.parse(created.item.expiresAt) - Date.parse(created.item.createdAt)).toBe(30 * 86_400_000)
  await expectDialogContained(settings, page)
  await expectMcpContentContained(settings)
  await expectNoHorizontalOverflow(page)
  await settings.getByRole("button", { name: "I saved it — hide token" }).click()
  await expect(secretField).toHaveCount(0)
  const createButton = settings.getByRole("button", { name: "Create token", exact: true })
  const nameField = settings.getByRole("textbox", { name: /^Token name/u })
  await expect(nameField).toHaveCount(0)
  await expect(createButton).toBeFocused()
  await createButton.click()
  await expect(nameField).toBeFocused()
  await expectMcpContentContained(settings)
  await settings.getByRole("button", { name: "Cancel", exact: true }).click()
  await expect(nameField).toHaveCount(0)
  await expect(createButton).toBeFocused()

  await settings.getByText("Connect external AI", { exact: true }).click()
  const configuration = settings.getByLabel("Example MCP client configuration")
  await expect(configuration).toContainText(`${server.baseURL}/mcp`)
  await expect(configuration).toContainText("Bearer <YOUR_TOKEN>")
  expect((await configuration.textContent())?.includes(created.token)).toBe(false)
  await expectDialogContained(settings, page)
  await expectMcpContentContained(settings)
  await expectNoHorizontalOverflow(page)

  const cookieOnly = await page.request.post(`${server.baseURL}/mcp`, {
    headers: { Accept: "application/json, text/event-stream", "MCP-Protocol-Version": protocolVersion },
    data: { jsonrpc: "2.0", id: 1, method: "ping" },
  })
  expect(cookieOnly.status()).toBe(401)

  const transport = await playwright.request.newContext({ baseURL: server.baseURL })
  try {
    const initialized = await rpc(transport, created.token, "initialize", 1, {
      protocolVersion,
      capabilities: {},
      clientInfo: { name: "raindrop-playwright", version: "1.0.0" },
    })
    expect(initialized.status()).toBe(200)
    expect(await initialized.json()).toMatchObject({
      jsonrpc: "2.0", id: 1,
      result: { protocolVersion, serverInfo: { name: "raindrop" }, capabilities: { tools: {} } },
    })
    const notification = await rpc(transport, created.token, "notifications/initialized")
    expect(notification.status()).toBe(202)

    const listed = await rpc(transport, created.token, "tools/list", 2)
    expect(listed.status()).toBe(200)
    const { result: { tools } } = await listed.json()
    expect(tools.map((tool: { name: string }) => tool.name).sort()).toEqual([
      "get_entry", "list_entries", "list_subscriptions",
    ])
    for (const tool of tools) {
      expect(tool.annotations).toMatchObject({ readOnlyHint: true, destructiveHint: false, openWorldHint: false })
    }

    for (const [index, name] of ["list_subscriptions", "list_entries"].entries()) {
      const called = await rpc(transport, created.token, "tools/call", index + 3, { name, arguments: { limit: 5 } })
      expect(called.status()).toBe(200)
      const { result } = await called.json()
      expect(result.isError).toBe(false)
      expect(result.content[0].type).toBe("text")
      expect(JSON.parse(result.content[0].text)).toMatchObject({ items: [], nextCursor: null })
    }
    const write = await rpc(transport, created.token, "tools/call", 5, { name: "mark_read", arguments: {} })
    expect(write.status()).toBe(200)
    expect(await write.json()).toMatchObject({ jsonrpc: "2.0", id: 5, error: { code: -32602 } })

    await settings.getByRole("button", { name: "Back to plugins" }).click()
    await settings.getByRole("button", { name: "Settings for MCP access", exact: true }).click()
    await expect(settings.getByRole("button", { name: `Revoke ${tokenName}`, exact: true })).toBeVisible()
    await expect(secretField).toHaveCount(0)
    const metadataResponse = await page.request.get(`${server.baseURL}/api/v1/mcp/tokens`)
    expect(metadataResponse.status()).toBe(200)
    const metadata = await metadataResponse.text()
    expect(metadata.includes(created.token)).toBe(false)
    const { items }: { items: McpToken[] } = JSON.parse(metadata)
    expect(items).toHaveLength(1)
    expect(items[0].id).toBe(created.item.id)
    expect(items[0].lastUsedAt).not.toBeNull()
    expect(Object.keys(items[0]).sort()).toEqual([
      "createdAt", "expiresAt", "id", "lastUsedAt", "name", "prefix",
    ])

    await settings.getByText("Connect external AI", { exact: true }).click()
    await expect(settings.getByLabel("Example MCP client configuration")).toBeVisible()
    await expectDialogContained(settings, page)
    await expectMcpContentContained(settings)
    await expectNoHorizontalOverflow(page)
    expect(await page.evaluate((secret) => document.documentElement.outerHTML.includes(secret), created.token)).toBe(false)
    if (process.env.RAINDROP_UPDATE_SCREENSHOTS === "1" && testInfo.project.use.viewport?.width !== 360) {
      if (testInfo.project.use.isMobile) {
        await settings.getByText("Connect external AI", { exact: true }).click()
      }
      await settings.locator(".reader-preferences-panel").evaluate((panel) => { panel.scrollTop = 0 })
      const heading = settings.getByRole("heading", { name: "MCP access", exact: true })
      await expect(heading).toBeInViewport()
      await heading.focus()
      if (testInfo.project.use.isMobile) {
        await settings.getByRole("region", { name: "Access credentials", exact: true }).scrollIntoViewIfNeeded()
        await expect(settings.getByRole("button", { name: `Revoke ${tokenName}`, exact: true })).toBeInViewport({ ratio: 1 })
      }
      await page.mouse.move(0, 0)
      await expectMcpContentContained(settings)
      const directory = new URL("../../docs/assets/screenshots/", import.meta.url)
      await mkdir(directory, { recursive: true })
      await page.screenshot({
        path: fileURLToPath(new URL(testInfo.project.use.isMobile ? "mcp-mobile.png" : "mcp-desktop.png", directory)),
        animations: "disabled",
        style: '[role="tooltip"] { visibility: hidden !important; }',
      })
    }

    const revoke = settings.getByRole("button", { name: `Revoke ${tokenName}`, exact: true })
    await revoke.click()
    const confirmation = page.getByRole("alertdialog", { name: "Revoke this token?" })
    await expectDialogContained(confirmation, page)
    await confirmation.getByRole("button", { name: "Cancel", exact: true }).click()
    const stillActive = await rpc(transport, created.token, "ping", 6)
    expect(stillActive.status()).toBe(200)
    await revoke.click()
    const deletion = page.waitForResponse((response) =>
      new URL(response.url()).pathname === `/api/v1/mcp/tokens/${created.item.id}` && response.request().method() === "DELETE",
    )
    await confirmation.getByRole("button", { name: "Revoke", exact: true }).click()
    expect((await deletion).status()).toBe(204)
    await expect(confirmation).not.toBeVisible()
    await expect(revoke).toHaveCount(0)
    await expect(settings.getByText("Token revoked. Clients can no longer use this credential.")).toBeVisible()
    const revoked = await rpc(transport, created.token, "tools/call", 7, { name: "list_entries", arguments: {} })
    expect(revoked.status()).toBe(401)
    expect(revoked.headers()["www-authenticate"]).toBe("Bearer")
    const remaining = await page.request.get(`${server.baseURL}/api/v1/mcp/tokens`)
    expect(remaining.status()).toBe(200)
    expect(await remaining.json()).toEqual({ items: [] })
    await expectNoHorizontalOverflow(page)
    expect(pageErrors).toEqual([])
  } finally {
    await transport.dispose()
  }

  if (process.env.RAINDROP_UPDATE_SCREENSHOTS === "1" && testInfo.project.use.viewport?.width === 1280) {
    await settings.getByRole("button", { name: "Close", exact: true }).click()
    await expect(settings).not.toBeVisible()
    await installReaderApiFixture(page)
    await page.goto(`${server.baseURL}/reader/feed/${readerIds.feedA}`, { waitUntil: "domcontentloaded" })
    await expect(page.getByRole("toolbar", { name: "Queue actions" })).toContainText("Quiet Web")
    await page.getByRole("button", { name: "Queue menu", exact: true }).click()
    await expect(page.getByRole("menuitem", { name: "Refresh Quiet Web", exact: true })).toBeVisible()
    await expect(page.getByRole("menuitem", { name: "Edit current subscription", exact: true })).toBeVisible()
    await expectNoHorizontalOverflow(page)
    await page.screenshot({
      path: fileURLToPath(new URL("../../docs/assets/screenshots/reader-subscription-actions.png", import.meta.url)),
      animations: "disabled",
    })
  }
})

async function openMcpSettings(page: Page) {
  const menu = page.getByRole("button", { name: "Open menu", exact: true })
  if (!(await menu.isVisible())) await page.getByRole("button", { name: "Open sources", exact: true }).click()
  await menu.click()
  await page.getByRole("menuitem", { name: "Settings", exact: true }).click()
  const settings = page.getByRole("dialog", { name: "Settings", exact: true })
  await settings.getByRole("button", { name: /^Plugins/u }).click()
  await settings.getByRole("button", { name: "Settings for MCP access", exact: true }).click()
  return settings
}

async function expectMcpContentContained(settings: Locator) {
  await expect.poll(() => settings.locator(".reader-mcp-settings").evaluate((section) => {
    const scrollport = section.closest<HTMLElement>(".reader-preferences-panel")
    if (!scrollport) return ["missing settings scrollport"]
    const bounds = scrollport.getBoundingClientRect()
    const sectionBounds = section.getBoundingClientRect()
    const left = Math.max(bounds.left + scrollport.clientLeft, sectionBounds.left)
    const right = Math.min(bounds.left + scrollport.clientLeft + scrollport.clientWidth, sectionBounds.right)
    return [section, ...section.querySelectorAll("*")].flatMap((element) => {
      const pre = element.closest("pre")
      if (pre && pre !== element) return []
      const box = element.getBoundingClientRect()
      if (box.width === 0 || box.height === 0 || getComputedStyle(element).visibility === "hidden") return []
      return box.left < left - 1 || box.right > right + 1
        ? [{ element: element.tagName, className: element.getAttribute("class"), left: box.left, right: box.right, scrollportLeft: left, scrollportRight: right }]
        : []
    })
  }), { message: "Every MCP element must fit the settings scrollport; only pre content may scroll horizontally" }).toEqual([])
}

async function rpc(transport: APIRequestContext, token: string, method: string, id?: number, params: object = {}) {
  try {
    return await transport.post("/mcp", {
      headers: {
        Authorization: `Bearer ${token}`,
        Accept: "application/json, text/event-stream",
        "MCP-Protocol-Version": protocolVersion,
      },
      data: { jsonrpc: "2.0", ...(id === undefined ? {} : { id }), method, params },
    })
  } catch {
    throw new Error(`MCP ${method} transport failed; protected request details omitted`)
  }
}
