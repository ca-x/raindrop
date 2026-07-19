import { afterEach, expect, it, vi } from "vitest"

import { commitOpml, exportOpml, previewOpml } from "./api"

const summary = {
  mode: "PREVIEW",
  outlineCount: 4,
  validCount: 2,
  newCount: 2,
  importedCount: 0,
  duplicateCount: 1,
  invalidCount: 1,
  categoryCount: 1,
  createdCategoryCount: 0,
} as const

afterEach(() => vi.unstubAllGlobals())

it("sends raw XML with CSRF for preview and commit", async () => {
  const fetchMock = vi
    .fn()
    .mockResolvedValueOnce(jsonResponse(summary))
    .mockResolvedValueOnce(
      jsonResponse({
        ...summary,
        mode: "COMMIT",
        importedCount: 2,
      }),
    )
  vi.stubGlobal("fetch", fetchMock)
  const file = new File(["<opml/>"] , "subscriptions.opml", {
    type: "application/xml",
  })

  await previewOpml(file, "csrf-memory")
  await commitOpml(file, "csrf-memory")

  for (const [path, init] of fetchMock.mock.calls) {
    expect(path).toMatch(/^\/api\/v1\/imports\/opml\?mode=/u)
    expect(new Headers(init?.headers).get("content-type")).toBe("application/xml")
    expect(new Headers(init?.headers).get("x-csrf-token")).toBe("csrf-memory")
    expect(init?.body).toBe(file)
  }
})

it("returns the OPML blob and attachment filename", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue(
      new Response("<opml/>", {
        status: 200,
        headers: {
          "content-type": "application/xml",
          "content-disposition": 'attachment; filename="raindrop.opml"',
        },
      }),
    ),
  )

  const exported = await exportOpml()

  expect(exported.filename).toBe("raindrop.opml")
  expect(await exported.blob.text()).toBe("<opml/>")
})

function jsonResponse(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "content-type": "application/json" },
  })
}
