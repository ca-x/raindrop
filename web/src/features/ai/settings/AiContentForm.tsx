import { Banner } from "@astryxdesign/core/Banner"
import { Button } from "@astryxdesign/core/Button"
import { CheckboxInput } from "@astryxdesign/core/CheckboxInput"
import { NumberInput } from "@astryxdesign/core/NumberInput"
import {
  SegmentedControl,
  SegmentedControlItem,
} from "@astryxdesign/core/SegmentedControl"
import { Selector } from "@astryxdesign/core/Selector"
import { Stack } from "@astryxdesign/core/Stack"
import { useLingui } from "@lingui/react"
import { useEffect, useMemo, useState, type FormEvent } from "react"

import type {
  AiConfigEnvelope,
  AiSummaryStyle,
  PutAiConfigRequest,
} from "../api/content.generated"
import type { Provider } from "../api/provider.generated"

interface AiContentFormProps {
  providers: Provider[]
  envelope: AiConfigEnvelope
  isSaving: boolean
  onSave: (request: PutAiConfigRequest) => Promise<boolean>
}

interface AiContentDraft {
  summaryEnabled: boolean
  summaryProviderId: string
  summaryStyle: AiSummaryStyle
  summaryMaxOutputTokens: number
  translationEnabled: boolean
  translationProviderId: string
  defaultTargetLocale: string
  translationMaxOutputTokens: number
}

const TARGET_LOCALES = ["zh-CN", "en", "ja-JP", "ko-KR", "fr", "de", "es"]

export function AiContentForm(props: AiContentFormProps) {
  const { i18n } = useLingui()
  const enabledProviders = props.providers.filter((provider) => provider.isEnabled)
  const defaultProviderId = enabledProviders[0]?.providerId ?? ""
  const [draft, setDraft] = useState(() => toDraft(props.envelope, defaultProviderId))
  useEffect(() => {
    setDraft(toDraft(props.envelope, defaultProviderId))
  }, [props.envelope, defaultProviderId])

  const providerOptions = props.providers.map((provider) => ({
    value: provider.providerId,
    label: `${provider.displayName} · ${provider.model}`,
    disabled: !provider.isEnabled,
  }))
  const localeOptions = useMemo(() => {
    const locales = TARGET_LOCALES.includes(draft.defaultTargetLocale)
      ? TARGET_LOCALES
      : [draft.defaultTargetLocale, ...TARGET_LOCALES]
    return locales.filter(Boolean).map((locale) => ({
      value: locale,
      label: i18n._(`ai.locale.${locale}`),
    }))
  }, [draft.defaultTargetLocale, i18n])
  const pluginReady = props.envelope.pluginState === "READY"
  const hasProvider = defaultProviderId.length > 0
  const isDisabled = props.isSaving || !pluginReady || !hasProvider
  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (isDisabled || !draft.summaryProviderId || !draft.translationProviderId) return
    await props.onSave({
      expectedRevision: props.envelope.config?.revision ?? null,
      isEnabled: draft.summaryEnabled || draft.translationEnabled,
      summary: {
        enabled: draft.summaryEnabled,
        providerId: draft.summaryProviderId,
        style: draft.summaryStyle,
        maxOutputTokens: draft.summaryMaxOutputTokens,
      },
      translation: {
        enabled: draft.translationEnabled,
        providerId: draft.translationProviderId,
        defaultTargetLocale: draft.defaultTargetLocale,
        maxOutputTokens: draft.translationMaxOutputTokens,
      },
    })
  }

  return (
    <form onSubmit={submit} className="ai-content-form">
      <Stack gap={5}>
        <Banner
          status="info"
          title={i18n._("ai.mcpUnavailable")}
          description={i18n._("ai.mcpUnavailableDescription")}
        />
        {!hasProvider ? (
          <Banner
            status="warning"
            title={i18n._("ai.configNeedsProvider")}
            description={i18n._("ai.configNeedsProviderDescription")}
          />
        ) : null}
        <section className="ai-operation-section" aria-labelledby="ai-summary-heading">
          <div>
            <div id="ai-summary-heading" className="reader-preference-label">
              {i18n._("ai.summaryTitle")}
            </div>
            <div className="reader-preference-description">
              {i18n._("ai.summaryDescription")}
            </div>
          </div>
          <CheckboxInput
            label={i18n._("ai.summaryEnabled")}
            value={draft.summaryEnabled}
            onChange={(summaryEnabled) => setDraft({ ...draft, summaryEnabled })}
            isDisabled={isDisabled}
          />
          <Selector
            label={i18n._("ai.summaryProvider")}
            value={draft.summaryProviderId || undefined}
            options={providerOptions}
            onChange={(summaryProviderId) =>
              setDraft({ ...draft, summaryProviderId })
            }
            placeholder={i18n._("ai.selectProvider")}
            isDisabled={isDisabled}
            width="100%"
          />
          <SegmentedControl
            label={i18n._("ai.summaryStyle")}
            value={draft.summaryStyle}
            onChange={(summaryStyle) =>
              setDraft({ ...draft, summaryStyle: summaryStyle as AiSummaryStyle })
            }
            layout="fill"
            isDisabled={isDisabled}
          >
            <SegmentedControlItem
              value="CONCISE"
              label={i18n._("ai.summaryStyleConcise")}
            />
            <SegmentedControlItem
              value="BALANCED"
              label={i18n._("ai.summaryStyleBalanced")}
            />
            <SegmentedControlItem
              value="DETAILED"
              label={i18n._("ai.summaryStyleDetailed")}
            />
          </SegmentedControl>
          <NumberInput
            label={i18n._("ai.summaryTokenLimit")}
            value={draft.summaryMaxOutputTokens}
            min={128}
            max={4096}
            isIntegerOnly
            onChange={(summaryMaxOutputTokens) =>
              setDraft({ ...draft, summaryMaxOutputTokens })
            }
            isDisabled={isDisabled}
            width="100%"
          />
        </section>
        <section
          className="ai-operation-section"
          aria-labelledby="ai-translation-heading"
        >
          <div>
            <div id="ai-translation-heading" className="reader-preference-label">
              {i18n._("ai.translationTitle")}
            </div>
            <div className="reader-preference-description">
              {i18n._("ai.translationDescription")}
            </div>
          </div>
          <CheckboxInput
            label={i18n._("ai.translationEnabled")}
            value={draft.translationEnabled}
            onChange={(translationEnabled) =>
              setDraft({ ...draft, translationEnabled })
            }
            isDisabled={isDisabled}
          />
          <Selector
            label={i18n._("ai.translationProvider")}
            value={draft.translationProviderId || undefined}
            options={providerOptions}
            onChange={(translationProviderId) =>
              setDraft({ ...draft, translationProviderId })
            }
            placeholder={i18n._("ai.selectProvider")}
            isDisabled={isDisabled}
            width="100%"
          />
          <Selector
            label={i18n._("ai.translationLocale")}
            value={draft.defaultTargetLocale}
            options={localeOptions}
            onChange={(defaultTargetLocale) =>
              setDraft({ ...draft, defaultTargetLocale })
            }
            isDisabled={isDisabled}
            width="100%"
          />
          <NumberInput
            label={i18n._("ai.translationTokenLimit")}
            value={draft.translationMaxOutputTokens}
            min={256}
            max={16_384}
            isIntegerOnly
            onChange={(translationMaxOutputTokens) =>
              setDraft({ ...draft, translationMaxOutputTokens })
            }
            isDisabled={isDisabled}
            width="100%"
          />
        </section>
        <div className="reader-dialog-actions">
          <Button
            label={i18n._("ai.configSave")}
            type="submit"
            variant="primary"
            isLoading={props.isSaving}
            isDisabled={isDisabled}
          />
        </div>
      </Stack>
    </form>
  )
}

function toDraft(
  envelope: AiConfigEnvelope,
  defaultProviderId: string,
): AiContentDraft {
  const config = envelope.config
  return {
    summaryEnabled: config?.summary.enabled ?? false,
    summaryProviderId: config?.summary.providerId ?? defaultProviderId,
    summaryStyle: config?.summary.style ?? "BALANCED",
    summaryMaxOutputTokens: config?.summary.maxOutputTokens ?? 1024,
    translationEnabled: config?.translation.enabled ?? false,
    translationProviderId: config?.translation.providerId ?? defaultProviderId,
    defaultTargetLocale: config?.translation.defaultTargetLocale ?? "zh-CN",
    translationMaxOutputTokens: config?.translation.maxOutputTokens ?? 4096,
  }
}
