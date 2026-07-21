import { mkdtempSync, readFileSync, rmSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { spawnSync } from "node:child_process"
import { afterAll, afterEach, expect, it, vi } from "vitest"

import { getPreferences, patchPreferences } from "./preferences"

const temporaryDirectories: string[] = []

afterAll(() => {
  for (const directory of temporaryDirectories) {
    rmSync(directory, { force: true, recursive: true })
  }
})

afterEach(() => vi.unstubAllGlobals())

const preferences = {
  locale: "zh-CN",
  themeMode: "DARK",
  layoutDensity: "COMPACT",
  readingFontScale: 110,
  readingFontFamily: "SANS",
  readingCustomFontId: null,
  readingColorScheme: "PAPER",
  linkOpenMode: "CURRENT_TAB",
} as const

it("generates the strict preference DTOs and includes them in drift checking", () => {
  const outputRoot = mkdtempSync(join(tmpdir(), "raindrop-preference-types-"))
  temporaryDirectories.push(outputRoot)

  const generated = runGenerator("--output-root", outputRoot)
  expect(generated.status, generated.stderr).toBe(0)
  expect(generated.stdout).toContain("preferences.generated.ts")

  const outputPath = join(
    outputRoot,
    "src/features/preferences/api/preferences.generated.ts",
  )
  const output = readFileSync(outputPath, "utf8")
  expect(output).toContain("export interface UserPreferences")
  expect(output).toContain("export type UserPreferencesLocale = \"zh-CN\" | \"en\"")
  expect(output).toContain("export type UserPreferencesThemeMode")
  expect(output).toContain("export type UserPreferencesLayoutDensity")
  expect(output).toContain("export type PatchUserPreferencesRequest = {")
  expect(output).toContain("Object.keys(value).length >= 1")
  expect(output).toContain('value["readingFontScale"] >= 85')
  expect(output).toContain('value["readingFontScale"] <= 130')

  const clean = runGenerator("--check", "--output-root", outputRoot)
  expect(clean.status, clean.stderr).toBe(0)
})

it("gets preferences through the generated strict response validator", async () => {
  const fetchMock = vi.fn().mockResolvedValue(jsonResponse(preferences))
  vi.stubGlobal("fetch", fetchMock)

  await expect(getPreferences()).resolves.toEqual(preferences)
  expect(fetchMock).toHaveBeenCalledWith(
    "/api/v2/preferences",
    expect.objectContaining({ credentials: "same-origin" }),
  )
})

it.each([
  ["missing field", { locale: "en", themeMode: "SYSTEM", layoutDensity: "BALANCED" }],
  ["unknown field", { ...preferences, userId: "not-public" }],
  ["invalid locale", { ...preferences, locale: "fr" }],
  ["invalid theme", { ...preferences, themeMode: "AUTO" }],
  ["invalid density", { ...preferences, layoutDensity: "DENSE" }],
  ["scale below range", { ...preferences, readingFontScale: 84 }],
  ["scale above range", { ...preferences, readingFontScale: 131 }],
  ["fractional scale", { ...preferences, readingFontScale: 100.5 }],
])("rejects a malformed 2xx %s response", async (_name, body) => {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue(jsonResponse(body)))

  await expect(getPreferences()).rejects.toMatchObject({
    name: "ApiClientError",
    payload: { code: "INVALID_RESPONSE" },
  })
})

it("patches only supplied fields with CSRF and an AbortSignal", async () => {
  const fetchMock = vi.fn().mockResolvedValue(jsonResponse(preferences))
  vi.stubGlobal("fetch", fetchMock)
  const signal = new AbortController().signal

  await expect(
    patchPreferences("csrf-memory", { readingFontScale: 110 }, signal),
  ).resolves.toEqual(preferences)

  const [path, init] = fetchMock.mock.calls[0] ?? []
  expect(path).toBe("/api/v2/preferences")
  expect(init?.method).toBe("PATCH")
  expect(init?.signal).toBe(signal)
  expect(new Headers(init?.headers).get("x-csrf-token")).toBe("csrf-memory")
  expect(JSON.parse(String(init?.body))).toEqual({ readingFontScale: 110 })
})

it("rejects a malformed PATCH success response", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue(jsonResponse({ ...preferences, readingFontScale: "110" })),
  )

  await expect(
    patchPreferences("csrf", { themeMode: "DARK" }),
  ).rejects.toMatchObject({ payload: { code: "INVALID_RESPONSE" } })
})

function runGenerator(...args: string[]) {
  const result = spawnSync(
    process.execPath,
    ["scripts/generate-reader-types.mjs", ...args],
    {
      cwd: process.cwd(),
      encoding: "utf8",
    },
  )
  return {
    status: result.status,
    stdout: result.stdout,
    stderr: result.stderr,
  }
}

function jsonResponse(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "content-type": "application/json" },
  })
}
