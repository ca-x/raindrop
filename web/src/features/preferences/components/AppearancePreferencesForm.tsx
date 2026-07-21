import { Banner } from "@astryxdesign/core/Banner"
import { Button } from "@astryxdesign/core/Button"
import { FileInput } from "@astryxdesign/core/FileInput"
import {
  SegmentedControl,
  SegmentedControlItem,
} from "@astryxdesign/core/SegmentedControl"
import { Stack } from "@astryxdesign/core/Stack"
import { TextInput } from "@astryxdesign/core/TextInput"
import { useLingui } from "@lingui/react"
import { useState, type FormEvent, type ReactNode } from "react"

import type { UserProfile } from "../../profile/api/profile.generated"
import type {
  ProfileControllerError,
  ProfileFieldError,
} from "../../profile/model/useProfileController"
import type { UserFont, UserPreferences } from "../api/preferences.generated"
import type { PreferencesControllerError } from "../model/usePreferencesController"

interface PreferencesFormProps {
  value: UserPreferences
  isSaving: boolean
  error: PreferencesControllerError | null
  onChange: (patch: Partial<UserPreferences>) => void
  onSubmit: (event: FormEvent<HTMLFormElement>) => void
}

interface PersonalPreferencesFormProps extends PreferencesFormProps {
  profile: UserProfile
  profileError: ProfileControllerError | null
  profileFieldErrors: Partial<Record<"displayName" | "email", ProfileFieldError>>
  onProfileChange: (profile: UserProfile) => void
}

export function PersonalPreferencesForm(props: PersonalPreferencesFormProps) {
  const { i18n } = useLingui()
  return (
    <PreferencesFormShell {...props}>
      <section
        className="reader-settings-section"
        aria-labelledby="reader-account-heading"
      >
        <div className="reader-settings-section-heading">
          <div id="reader-account-heading" className="reader-preference-label">
            {i18n._("preferences.account")}
          </div>
          <div className="reader-preference-description">
            {i18n._("preferences.accountDescription")}
          </div>
        </div>
        {props.profileError ? (
          <Banner
            status="error"
            title={i18n._("preferences.profileSaveError")}
            description={i18n._("preferences.profileSaveErrorDescription")}
          />
        ) : null}
        <div className="reader-profile-fields">
          <div className="reader-readonly-field">
            <div className="reader-readonly-field-heading">
              <span className="reader-preference-label">
                {i18n._("preferences.username")}
              </span>
              <span className="reader-readonly-badge">
                {i18n._("preferences.readOnly")}
              </span>
            </div>
            <div className="reader-readonly-value">{props.profile.username}</div>
            <div className="reader-preference-description">
              {i18n._("preferences.usernameDescription")}
            </div>
          </div>
          <TextInput
            label={i18n._("preferences.displayName")}
            description={i18n._("preferences.displayNameDescription")}
            value={props.profile.displayName ?? ""}
            onChange={(displayName) =>
              props.onProfileChange({
                ...props.profile,
                displayName: displayName || null,
              })
            }
            htmlName="displayName"
            isOptional
            isDisabled={props.isSaving}
            width="100%"
            status={profileFieldStatus(
              props.profileFieldErrors.displayName,
              i18n._("preferences.displayNameInvalid"),
            )}
          />
          <TextInput
            label={i18n._("preferences.email")}
            description={i18n._("preferences.emailDescription")}
            type="email"
            value={props.profile.email ?? ""}
            onChange={(email) =>
              props.onProfileChange({
                ...props.profile,
                email: email || null,
              })
            }
            htmlName="email"
            isOptional
            isDisabled={props.isSaving}
            width="100%"
            status={profileFieldStatus(
              props.profileFieldErrors.email,
              i18n._(
                props.profileFieldErrors.email === "TAKEN"
                  ? "preferences.emailTaken"
                  : "preferences.emailInvalid",
              ),
            )}
          />
        </div>
      </section>
      <section
        className="reader-settings-section"
        aria-labelledby="reader-interface-heading"
      >
        <div className="reader-settings-section-heading">
          <div id="reader-interface-heading" className="reader-preference-label">
            {i18n._("preferences.interface")}
          </div>
          <div className="reader-preference-description">
            {i18n._("preferences.interfaceDescription")}
          </div>
        </div>
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
      </section>
    </PreferencesFormShell>
  )
}

function profileFieldStatus(error: ProfileFieldError | undefined, message: string) {
  return error ? ({ type: "error", message } as const) : undefined
}

interface ReadingPreferencesFormProps extends PreferencesFormProps {
  fonts: UserFont[]
  fontLimits: { maximumCount: number; maximumBytes: number }
  isFontMutating: boolean
  onUploadFont: (file: File) => Promise<boolean>
  onDeleteFont: (fontId: string) => Promise<boolean>
}

export function ReadingPreferencesForm(props: ReadingPreferencesFormProps) {
  const { i18n } = useLingui()
  return (
    <PreferencesFormShell {...props}>
      <CustomFontManagement {...props} />
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

function CustomFontManagement(props: ReadingPreferencesFormProps) {
  const { i18n } = useLingui()
  const [file, setFile] = useState<File | null>(null)
  const upload = async () => {
    if (!file) return
    if (await props.onUploadFont(file)) setFile(null)
  }
  return (
    <PreferenceField
      label={i18n._("preferences.customFonts")}
      description={i18n._("preferences.customFontsDescription", {
        count: props.fontLimits.maximumCount,
        size: Math.round(props.fontLimits.maximumBytes / 1024 / 1024),
      })}
    >
      <div className="reader-font-upload">
        <FileInput
          label={i18n._("preferences.customFontFile")}
          isLabelHidden
          value={file}
          onChange={(value) => setFile(value instanceof File ? value : null)}
          accept=".woff2,font/woff2"
          maxSize={props.fontLimits.maximumBytes}
          placeholder={i18n._("preferences.chooseCustomFont")}
          isDisabled={props.isFontMutating || props.fonts.length >= props.fontLimits.maximumCount}
        />
        <Button
          label={i18n._("preferences.uploadCustomFont")}
          onClick={() => void upload()}
          isLoading={props.isFontMutating}
          isDisabled={!file || props.fonts.length >= props.fontLimits.maximumCount}
          variant="secondary"
        />
      </div>
      <div className="reader-font-list" aria-label={i18n._("preferences.customFonts")}>
        {props.fonts.length === 0 ? (
          <div className="reader-preference-description">
            {i18n._("preferences.noCustomFonts")}
          </div>
        ) : props.fonts.map((font) => (
          <div key={font.fontId} className="reader-font-row">
            <div>
              <div className="reader-preference-label">{font.displayName}</div>
              <div className="reader-preference-description">{formatBytes(font.byteSize)}</div>
            </div>
            <Button
              label={i18n._("preferences.deleteCustomFont", { name: font.displayName })}
              onClick={() => void props.onDeleteFont(font.fontId)}
              isDisabled={props.isFontMutating}
              variant="destructive"
            />
          </div>
        ))}
      </div>
    </PreferenceField>
  )
}

function formatBytes(bytes: number): string {
  return bytes >= 1024 * 1024
    ? `${(bytes / 1024 / 1024).toFixed(1)} MiB`
    : `${Math.max(1, Math.round(bytes / 1024))} KiB`
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
