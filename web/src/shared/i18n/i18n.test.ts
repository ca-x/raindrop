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

  it("keeps page subtitles descriptive in both locales", () => {
    activateLocale("en")
    expect(i18n._("setup.contextTitle")).toBe("Database and administrator account")
    expect(i18n._("login.contextTitle")).toBe(
      "Unread articles and saved items in this instance",
    )
    expect(i18n._("preferences.description")).toBe(
      "Manage account, reading, plugins, and backups.",
    )
    expect(i18n._("plugins.translation.description")).toBe(
      "Translate full articles and look up words with OpenAI or DeepLX.",
    )

    activateLocale("zh-CN")
    expect(i18n._("setup.contextTitle")).toBe("数据库与管理员账户")
    expect(i18n._("login.contextTitle")).toBe("当前实例的未读文章与收藏")
    expect(i18n._("preferences.description")).toBe("管理账户、阅读、插件与备份设置。")
    expect(i18n._("plugins.translation.description")).toBe(
      "使用 OpenAI 或 DeepLX 翻译全文并查询词句。",
    )
  })
})
