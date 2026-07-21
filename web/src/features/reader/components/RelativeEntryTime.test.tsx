import { describe, expect, it } from "vitest"

import { formatRelativeEntryTime } from "./RelativeEntryTime"

describe("formatRelativeEntryTime", () => {
  const now = Date.UTC(2026, 6, 21, 12, 0, 0)

  it("formats recent entries with stable relative units", () => {
    expect(formatRelativeEntryTime(now - 20_000, now, "en", "Just now")).toBe("Just now")
    expect(formatRelativeEntryTime(now - 5 * 60_000, now, "en", "Just now")).toBe("5 minutes ago")
    expect(formatRelativeEntryTime(now - 3 * 60 * 60_000, now, "en", "Just now")).toBe("3 hours ago")
    expect(formatRelativeEntryTime(now - 4 * 24 * 60 * 60_000, now, "en", "Just now")).toBe("4 days ago")
  })

  it("uses the requested locale and supports future timestamps", () => {
    expect(formatRelativeEntryTime(now - 2 * 60 * 60_000, now, "zh-CN", "刚刚")).toBe("2小时前")
    expect(formatRelativeEntryTime(now + 10 * 60_000, now, "en", "Just now")).toBe("in 10 minutes")
  })
})
