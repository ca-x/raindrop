import { describe, expect, it } from "vitest"

import { activateLocale, i18n } from "./i18n"

describe("production-safe message catalogs", () => {
  it("loads static messages in compiled form", () => {
    activateLocale("en")

    expect(i18n.messages["app.loading"]).toEqual(["Preparing Raindrop"])
    expect(i18n._("app.loading")).toBe("Preparing Raindrop")
  })

  it("interpolates compiled English and Chinese placeholders", () => {
    activateLocale("en")
    expect(i18n._("reader.newEntriesAvailable", { count: 3 })).toBe(
      "3 new entries available",
    )
    expect(i18n._("reader.refreshFeed", { title: "IT Home" })).toBe(
      "Refresh IT Home",
    )
    expect(i18n._("reader.refreshQueued")).toBe("Queued for refresh")
    expect(i18n._("reader.refreshRunning")).toBe("Refreshing")

    activateLocale("zh-CN")
    expect(i18n._("reader.newEntriesAvailable", { count: 3 })).toBe(
      "有 3 篇新文章可用",
    )
    expect(i18n._("reader.refreshFeed", { title: "IT之家" })).toBe(
      "刷新 IT之家",
    )
    expect(i18n._("reader.refreshQueued")).toBe("已加入刷新队列")
    expect(i18n._("reader.refreshRunning")).toBe("正在刷新")
  })
})
