import { Icon } from "@astryxdesign/core/Icon"
import { SegmentedControl, SegmentedControlItem } from "@astryxdesign/core/SegmentedControl"
import { Switch } from "@astryxdesign/core/Switch"
import { useLingui } from "@lingui/react"

import type { UserPreferences } from "../api/preferences.generated"
import {
  PreferenceField,
  PreferencesFormShell,
  ReadingColorPreview,
  type PreferencesFormProps,
} from "./AppearancePreferencesForm"

const themes = [
  { value: "LIGHT", label: "preferences.themeLight" },
  { value: "DARK", label: "preferences.themeDark" },
  { value: "SYSTEM", label: "preferences.themeSystem" },
] as const

export function InterfacePreferencesForm(props: PreferencesFormProps) {
  const { i18n } = useLingui()
  return (
    <PreferencesFormShell {...props}>
      <PreferenceField
        label={i18n._("preferences.appearance")}
        description={i18n._("preferences.appearanceDescription")}
      >
        <SegmentedControl
          label={i18n._("preferences.appearance")}
          value={props.value.themeMode}
          onChange={(themeMode) => props.onChange({ themeMode: themeMode as UserPreferences["themeMode"] })}
          isDisabled={props.isSaving}
          layout="fill"
          className="reader-theme-options"
        >
          {themes.map(({ value, label }) => (
            <SegmentedControlItem
              key={value}
              value={value}
              label={i18n._(label)}
              icon={<ThemeThumbnail mode={value} />}
            />
          ))}
        </SegmentedControl>
      </PreferenceField>
      <div className="reader-appearance-preview-frame">
        <div className="reader-preview-caption">
          <span>{i18n._("preferences.livePreview")}</span>
          <span>{i18n._("preferences.previewBeforeSave")}</span>
        </div>
        <div
          className="reader-appearance-preview"
          data-mode={props.value.themeMode.toLowerCase()}
          data-raindrop-blur={props.value.interfaceBlur}
          data-raindrop-shadows={props.value.interfaceShadows}
        >
          <div className="reader-preview-sidebar" aria-hidden="true">
            <Icon icon="viewColumns" size="sm" />
            <span className="reader-preview-line" />
            <span className="reader-preview-line" />
            <span className="reader-preview-line" />
          </div>
          <div className="reader-preview-page">
            <ReadingColorPreview value={props.value.readingColorScheme} />
            <div className="reader-preview-material" aria-hidden="true">
              <Icon icon="check" size="sm" />
              {i18n._("preferences.previewToolbar")}
              <span className="reader-preview-material-divider" />
              Aa
            </div>
          </div>
        </div>
      </div>
      <section className="reader-settings-section" aria-labelledby="reader-material-heading">
        <div className="reader-settings-section-heading">
          <div id="reader-material-heading" className="reader-preference-label">{i18n._("preferences.materials")}</div>
          <div className="reader-preference-description">{i18n._("preferences.materialsDescription")}</div>
        </div>
        <div className="reader-material-controls">
          <Switch
            label={i18n._("preferences.interfaceBlur")}
            description={i18n._("preferences.interfaceBlurDescription")}
            value={props.value.interfaceBlur}
            onChange={(interfaceBlur) => props.onChange({ interfaceBlur })}
            isDisabled={props.isSaving}
            labelPosition="start"
            labelSpacing="spread"
            width="100%"
          />
          <Switch
            label={i18n._("preferences.interfaceShadows")}
            description={i18n._("preferences.interfaceShadowsDescription")}
            value={props.value.interfaceShadows}
            onChange={(interfaceShadows) => props.onChange({ interfaceShadows })}
            isDisabled={props.isSaving}
            labelPosition="start"
            labelSpacing="spread"
            width="100%"
          />
        </div>
      </section>
      <PreferenceField label={i18n._("preferences.density")} description={i18n._("preferences.densityDescription")}>
        <SegmentedControl
          label={i18n._("preferences.density")}
          value={props.value.layoutDensity}
          onChange={(layoutDensity) => props.onChange({ layoutDensity: layoutDensity as UserPreferences["layoutDensity"] })}
          layout="fill"
          isDisabled={props.isSaving}
        >
          <SegmentedControlItem value="COMPACT" label={i18n._("preferences.densityCompact")} />
          <SegmentedControlItem value="BALANCED" label={i18n._("preferences.densityBalanced")} />
          <SegmentedControlItem value="SPACIOUS" label={i18n._("preferences.densitySpacious")} />
        </SegmentedControl>
      </PreferenceField>
    </PreferencesFormShell>
  )
}

function ThemeThumbnail({ mode }: { mode: UserPreferences["themeMode"] }) {
  return (
    <span className="reader-theme-thumbnail" data-mode={mode.toLowerCase()} aria-hidden="true">
      <span className="reader-theme-thumbnail-sidebar"><span /><span /><span /></span>
      <span className="reader-theme-thumbnail-page"><span /><span /><span /><span /></span>
      <span className="reader-theme-thumbnail-check"><Icon icon="check" size="xsm" /></span>
    </span>
  )
}
