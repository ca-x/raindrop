import {
  SegmentedControl,
  SegmentedControlItem,
} from "@astryxdesign/core/SegmentedControl"
import { useLingui } from "@lingui/react"
import type { CSSProperties } from "react"

import { activateLocale, type AppLocale } from "./i18n"

interface LocaleSwitchProps {
  isDisabled?: boolean
}

const touchTargetStyle = {
  "--size-element-md": "48px",
} as CSSProperties

export function LocaleSwitch({ isDisabled = false }: LocaleSwitchProps) {
  const { i18n } = useLingui()
  return (
    <SegmentedControl
      value={i18n.locale}
      onChange={(locale) => activateLocale(locale as AppLocale)}
      label={i18n._("common.language")}
      size="md"
      isDisabled={isDisabled}
      style={touchTargetStyle}
    >
      <SegmentedControlItem value="zh-CN" label="中文" />
      <SegmentedControlItem value="en" label="English" />
    </SegmentedControl>
  )
}
