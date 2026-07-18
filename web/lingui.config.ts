import type { LinguiConfig } from "@lingui/conf"
import { formatter } from "@lingui/format-po"

const config: LinguiConfig = {
  locales: ["zh-CN", "en"],
  sourceLocale: "en",
  catalogs: [
    {
      include: ["<rootDir>/src"],
      path: "<rootDir>/src/locales/{locale}/messages",
    },
  ],
  format: formatter(),
}

export default config
