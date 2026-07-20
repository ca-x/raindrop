import { Banner } from "@astryxdesign/core/Banner"
import {
  SegmentedControl,
  SegmentedControlItem,
} from "@astryxdesign/core/SegmentedControl"
import { Slider } from "@astryxdesign/core/Slider"
import { Stack } from "@astryxdesign/core/Stack"
import { useLingui } from "@lingui/react"
import type { FormEvent, ReactNode } from "react"

import type { UserPreferences } from "../api/preferences.generated"
import type { PreferencesControllerError } from "../model/usePreferencesController"

interface PreferencesFormProps {
  value: UserPreferences
  isSaving: boolean
  error: PreferencesControllerError | null
  onChange: (patch: Partial<UserPreferences>) => void
  onSubmit: (event: FormEvent<HTMLFormElement>) => void
}

interface PersonalPreferencesFormProps extends PreferencesFormProps {
  account: { username: string; email: string | null }
}

export function PersonalPreferencesForm(props: PersonalPreferencesFormProps) {
  const { i18n } = useLingui()
  return (
    <PreferencesFormShell {...props}>
      <section className="reader-account-card" aria-labelledby="reader-account-heading">
        <div>
          <div id="reader-account-heading" className="reader-preference-label">
            {i18n._("preferences.account")}
          </div>
          <div className="reader-preference-description">
            {i18n._("preferences.accountDescription")}
          </div>
        </div>
        <dl className="reader-account-details">
          <div>
            <dt>{i18n._("preferences.username")}</dt>
            <dd>{props.account.username}</dd>
          </div>
          <div>
            <dt>{i18n._("preferences.email")}</dt>
            <dd>{props.account.email ?? i18n._("preferences.emailUnset")}</dd>
          </div>
        </dl>
      </section>
      <PreferenceField
        label={i18n._("preferences.appearance")}
        description={i18n._("preferences.appearanceDescription")}
      >
        <SegmentedControl
          label={i18n._("preferences.appearance")}
          value={props.value.themeMode}
          onChange={(value) =>
            props.onChange({ themeMode: value as UserPreferences["themeMode"] })
          }
          layout="fill"
          isDisabled={props.isSaving}
        >
          <SegmentedControlItem value="SYSTEM" label={i18n._("preferences.themeSystem")} />
          <SegmentedControlItem value="LIGHT" label={i18n._("preferences.themeLight")} />
          <SegmentedControlItem value="DARK" label={i18n._("preferences.themeDark")} />
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
          <SegmentedControlItem value="COMPACT" label={i18n._("preferences.densityCompact")} />
          <SegmentedControlItem value="BALANCED" label={i18n._("preferences.densityBalanced")} />
          <SegmentedControlItem value="SPACIOUS" label={i18n._("preferences.densitySpacious")} />
        </SegmentedControl>
      </PreferenceField>
    </PreferencesFormShell>
  )
}

export function ReadingPreferencesForm(props: PreferencesFormProps) {
  const { i18n } = useLingui()
  return (
    <PreferencesFormShell {...props}>
      <PreferenceField
        label={i18n._("preferences.readingFont")}
        description={i18n._("preferences.readingFontDescription")}
      >
        <SegmentedControl
          label={i18n._("preferences.readingFont")}
          value={props.value.readingFontFamily}
          onChange={(value) =>
            props.onChange({
              readingFontFamily: value as UserPreferences["readingFontFamily"],
            })
          }
          layout="fill"
          isDisabled={props.isSaving}
        >
          <SegmentedControlItem value="SERIF" label={i18n._("preferences.fontSerif")} />
          <SegmentedControlItem value="SANS" label={i18n._("preferences.fontSans")} />
        </SegmentedControl>
      </PreferenceField>
      <PreferenceField
        label={i18n._("preferences.readingColor")}
        description={i18n._("preferences.readingColorDescription")}
      >
        <SegmentedControl
          label={i18n._("preferences.readingColor")}
          value={props.value.readingColorScheme}
          onChange={(value) =>
            props.onChange({
              readingColorScheme: value as UserPreferences["readingColorScheme"],
            })
          }
          layout="fill"
          isDisabled={props.isSaving}
        >
          <SegmentedControlItem value="AUTO" label={i18n._("preferences.colorAuto")} />
          <SegmentedControlItem value="PAPER" label={i18n._("preferences.colorPaper")} />
          <SegmentedControlItem value="SEPIA" label={i18n._("preferences.colorSepia")} />
          <SegmentedControlItem value="GRAY" label={i18n._("preferences.colorGray")} />
        </SegmentedControl>
      </PreferenceField>
      <PreferenceField
        label={i18n._("preferences.readingSize")}
        description={i18n._("preferences.readingSizeDescription")}
      >
        <Slider
          label={i18n._("preferences.readingSize")}
          isLabelHidden
          value={props.value.readingFontScale}
          min={85}
          max={130}
          step={5}
          formatValue={(value) => `${value}%`}
          valueDisplay="text"
          onChange={(readingFontScale: number) => props.onChange({ readingFontScale })}
          isDisabled={props.isSaving}
          width="100%"
        />
      </PreferenceField>
      <PreferenceField
        label={i18n._("preferences.linkOpenMode")}
        description={i18n._("preferences.linkOpenModeDescription")}
      >
        <SegmentedControl
          label={i18n._("preferences.linkOpenMode")}
          value={props.value.linkOpenMode}
          onChange={(value) =>
            props.onChange({ linkOpenMode: value as UserPreferences["linkOpenMode"] })
          }
          layout="fill"
          isDisabled={props.isSaving}
        >
          <SegmentedControlItem value="CURRENT_TAB" label={i18n._("preferences.linkCurrent")} />
          <SegmentedControlItem value="NEW_TAB" label={i18n._("preferences.linkNewTab")} />
        </SegmentedControl>
      </PreferenceField>
    </PreferencesFormShell>
  )
}

function PreferencesFormShell(
  props: PreferencesFormProps & { children: ReactNode },
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
          <Banner status="error" title={errorCopy.title} description={errorCopy.description} />
        ) : null}
        {props.children}
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
