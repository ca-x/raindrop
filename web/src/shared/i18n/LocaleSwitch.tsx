import {
  SegmentedControl,
  SegmentedControlItem,
} from "@astryxdesign/core/SegmentedControl"
import { useLingui } from "@lingui/react"

import { activateLocale, type AppLocale } from "./i18n"

export function LocaleSwitch() {
  const { i18n } = useLingui()
  return (
    <SegmentedControl
      value={i18n.locale}
      onChange={(locale) => activateLocale(locale as AppLocale)}
      label={i18n._("common.language")}
      size="sm"
    >
      <SegmentedControlItem value="zh-CN" label="中文" />
      <SegmentedControlItem value="en" label="English" />
    </SegmentedControl>
  )
}
