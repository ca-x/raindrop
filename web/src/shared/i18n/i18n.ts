import { setupI18n } from "@lingui/core"

export type AppLocale = "zh-CN" | "en"

const catalogs: Record<AppLocale, Record<string, string>> = {
  "zh-CN": {
    "app.loading": "正在准备 Raindrop",
    "app.loadError": "无法加载 Raindrop",
    "app.loadErrorDescription": "请检查服务连接后刷新页面。",
    "setup.eyebrow": "首次启动",
    "setup.title": "开始设置 Raindrop",
    "setup.description": "连接数据库并创建首位管理员。设置完成后，此入口会自动关闭。",
    "setup.continue": "继续设置",
    "login.eyebrow": "Raindrop 阅读器",
    "login.title": "欢迎回来",
    "login.description": "登录后继续处理未读文章。",
    "login.continue": "登录",
    "ready.title": "阅读空间已就绪",
    "ready.description": "订阅与文章功能将在下一阶段接入。",
  },
  en: {
    "app.loading": "Preparing Raindrop",
    "app.loadError": "Raindrop could not load",
    "app.loadErrorDescription": "Check the service connection and refresh the page.",
    "setup.eyebrow": "First run",
    "setup.title": "Set up Raindrop",
    "setup.description":
      "Connect a database and create the first administrator. This entry closes when setup finishes.",
    "setup.continue": "Continue setup",
    "login.eyebrow": "Raindrop reader",
    "login.title": "Welcome back",
    "login.description": "Sign in to continue with your unread articles.",
    "login.continue": "Sign in",
    "ready.title": "Your reading space is ready",
    "ready.description": "Subscriptions and articles arrive in the next foundation slice.",
  },
}

export const i18n = setupI18n()

export function activateLocale(locale: AppLocale) {
  i18n.load(locale, catalogs[locale])
  i18n.activate(locale)
  document.documentElement.lang = locale
}

export function detectLocale(): AppLocale {
  return navigator.language.toLowerCase().startsWith("zh") ? "zh-CN" : "en"
}

activateLocale(detectLocale())
