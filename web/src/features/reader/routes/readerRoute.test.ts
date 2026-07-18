import { describe, expect, it } from "vitest"

import { parseReaderPath, pathForSource, sameReaderSource } from "./readerRoute"

describe("parseReaderPath", () => {
  it("rejects malformed feed percent encoding without throwing", () => {
    expect(() => parseReaderPath("/reader/feed/%E0%A4%A")).not.toThrow()
    expect(parseReaderPath("/reader/feed/%E0%A4%A")).toBeNull()
  })

  it("rejects malformed entry percent encoding without throwing", () => {
    expect(() => parseReaderPath("/reader/unread/entry/%E0%A4%A")).not.toThrow()
    expect(parseReaderPath("/reader/unread/entry/%E0%A4%A")).toBeNull()
  })

  it("round-trips encoded category sources and entry deep links", () => {
    const category = { kind: "category", categoryId: "category/one" } as const
    expect(pathForSource(category)).toBe("/reader/category/category%2Fone")
    expect(parseReaderPath("/reader/category/category%2Fone")).toEqual({
      source: category,
      sourcePath: "/reader/category/category%2Fone",
      entryId: null,
    })
    expect(parseReaderPath("/reader/category/category%2Fone/entry/entry%2Fone")).toEqual({
      source: category,
      sourcePath: "/reader/category/category%2Fone",
      entryId: "entry/one",
    })
    expect(sameReaderSource(category, { ...category })).toBe(true)
    expect(
      sameReaderSource(category, { kind: "category", categoryId: "category/two" }),
    ).toBe(false)
  })
})
