import { Banner } from "@astryxdesign/core/Banner"
import {
  SegmentedControl,
  SegmentedControlItem,
} from "@astryxdesign/core/SegmentedControl"
import { Stack } from "@astryxdesign/core/Stack"
import { useLingui } from "@lingui/react"
import type { FormEvent, ReactNode } from "react"

import type { UserPreferences } from "../api/preferences.generated"
import type { PreferencesControllerError } from "../model/usePreferencesController"

interface AppearancePreferencesFormProps {
  value: UserPreferences
  isSaving: boolean
  error: PreferencesControllerError | null
  onChange: (patch: Partial<UserPreferences>) => void
  onSubmit: (event: FormEvent<HTMLFormElement>) => void
}

export function AppearancePreferencesForm(
  props: AppearancePreferencesFormProps,
) {
  const { i18n } = useLingui()
  const errorCopy = props.error
    ? {
        title: i18n._(
          props.error === "LOAD" ? "preferences.loadError" : "preferences.saveError",
        ),
        description: i18n._(
          props.error === "LOAD"
            ? "preferences.loadErrorDescription"
            : "preferences.saveErrorDescription",
        ),
      }
    : null

  return (
    <form id="reader-preferences-form" onSubmit={props.onSubmit}>
      <Stack gap={5}>
        {errorCopy ? (
          <Banner
            status="error"
            title={errorCopy.title}
            description={errorCopy.description}
          />
        ) : null}
        <PreferenceField
          label={i18n._("preferences.appearance")}
          description={i18n._("preferences.appearanceDescription")}
        >
          <SegmentedControl
            label={i18n._("preferences.appearance")}
            value={props.value.themeMode}
            onChange={(value) =>
              props.onChange({
                themeMode: value as UserPreferences["themeMode"],
              })
            }
            layout="fill"
            isDisabled={props.isSaving}
          >
            <SegmentedControlItem
              value="SYSTEM"
              label={i18n._("preferences.themeSystem")}
            />
            <SegmentedControlItem
              value="LIGHT"
              label={i18n._("preferences.themeLight")}
            />
            <SegmentedControlItem
              value="DARK"
              label={i18n._("preferences.themeDark")}
            />
          </SegmentedControl>
        </PreferenceField>
        <PreferenceField
          label={i18n._("preferences.language")}
          description={i18n._("preferences.languageDescription")}
        >
          <SegmentedControl
            label={i18n._("preferences.language")}
            value={props.value.locale}
            onChange={(value) =>
              props.onChange({ locale: value as UserPreferences["locale"] })
            }
            layout="fill"
            isDisabled={props.isSaving}
          >
            <SegmentedControlItem value="zh-CN" label="中文" />
            <SegmentedControlItem value="en" label="English" />
          </SegmentedControl>
        </PreferenceField>
        <PreferenceField
          label={i18n._("preferences.density")}
          description={i18n._("preferences.densityDescription")}
        >
          <SegmentedControl
            label={i18n._("preferences.density")}
            value={props.value.layoutDensity}
            onChange={(value) =>
              props.onChange({
                layoutDensity: value as UserPreferences["layoutDensity"],
              })
            }
            layout="fill"
            isDisabled={props.isSaving}
          >
            <SegmentedControlItem
              value="COMPACT"
              label={i18n._("preferences.densityCompact")}
            />
            <SegmentedControlItem
              value="BALANCED"
              label={i18n._("preferences.densityBalanced")}
            />
            <SegmentedControlItem
              value="SPACIOUS"
              label={i18n._("preferences.densitySpacious")}
            />
          </SegmentedControl>
        </PreferenceField>
        <PreferenceField
          label={i18n._("preferences.readingSize")}
          description={i18n._("preferences.readingSizeDescription")}
        >
          <SegmentedControl
            label={i18n._("preferences.readingSize")}
            value={String(props.value.readingFontScale)}
            onChange={(value) =>
              props.onChange({ readingFontScale: Number(value) })
            }
            layout="fill"
            isDisabled={props.isSaving}
          >
            {[90, 100, 110, 120].map((scale) => (
              <SegmentedControlItem
                key={scale}
                value={String(scale)}
                label={`${scale}%`}
              />
            ))}
          </SegmentedControl>
        </PreferenceField>
      </Stack>
    </form>
  )
}

function PreferenceField({
  label,
  description,
  children,
}: {
  label: string
  description: string
  children: ReactNode
}) {
  return (
    <Stack gap={2} className="reader-preference-field">
      <div>
        <div className="reader-preference-label">{label}</div>
        <div className="reader-preference-description">{description}</div>
      </div>
      {children}
    </Stack>
  )
}
