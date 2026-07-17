import { describe, expect, it } from "vitest"

import { parseReaderPath } from "./readerRoute"

describe("parseReaderPath", () => {
  it("rejects malformed feed percent encoding without throwing", () => {
    expect(() => parseReaderPath("/reader/feed/%E0%A4%A")).not.toThrow()
    expect(parseReaderPath("/reader/feed/%E0%A4%A")).toBeNull()
  })

  it("rejects malformed entry percent encoding without throwing", () => {
    expect(() => parseReaderPath("/reader/unread/entry/%E0%A4%A")).not.toThrow()
    expect(parseReaderPath("/reader/unread/entry/%E0%A4%A")).toBeNull()
  })
})
