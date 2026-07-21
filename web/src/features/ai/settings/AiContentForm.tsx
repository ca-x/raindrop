import { Banner } from "@astryxdesign/core/Banner"
import { Button } from "@astryxdesign/core/Button"
import { NumberInput } from "@astryxdesign/core/NumberInput"
import {
  SegmentedControl,
  SegmentedControlItem,
} from "@astryxdesign/core/SegmentedControl"
import { Selector } from "@astryxdesign/core/Selector"
import { Stack } from "@astryxdesign/core/Stack"
import { Switch } from "@astryxdesign/core/Switch"
import { useLingui } from "@lingui/react"
import { useEffect, useState, type FormEvent } from "react"

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
  pluginEnabled: boolean
  summaryEnabled: boolean
  summaryProviderId: string
  summaryStyle: AiSummaryStyle
  summaryMaxOutputTokens: number
}

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
  const pluginReady = props.envelope.pluginState === "READY"
  const hasProvider = defaultProviderId.length > 0
  const isUnavailable = props.isSaving || !pluginReady
  const pluginToggleDisabled = isUnavailable || (!hasProvider && !draft.pluginEnabled)
  const operationsDisabled = isUnavailable || !hasProvider || !draft.pluginEnabled
  const canSubmit =
    !isUnavailable &&
    Boolean(draft.summaryProviderId) &&
    (hasProvider || !draft.pluginEnabled)
  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (!canSubmit) return
    await props.onSave({
      expectedRevision: props.envelope.config?.revision ?? null,
      isEnabled: draft.pluginEnabled,
      summary: {
        enabled: draft.summaryEnabled,
        providerId: draft.summaryProviderId,
        style: draft.summaryStyle,
        maxOutputTokens: draft.summaryMaxOutputTokens,
      },
      translation: {
        enabled: false,
        providerId:
          props.envelope.config?.translation.providerId ?? draft.summaryProviderId,
        defaultTargetLocale:
          props.envelope.config?.translation.defaultTargetLocale ?? "zh-CN",
        maxOutputTokens:
          props.envelope.config?.translation.maxOutputTokens ?? 4096,
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
        <section className="ai-plugin-toggle" aria-labelledby="ai-plugin-heading">
          <div>
            <div id="ai-plugin-heading" className="reader-preference-label">
              {i18n._("ai.pluginTitle")}
            </div>
            <div className="reader-preference-description">
              {i18n._("ai.pluginDescription")}
            </div>
          </div>
          <Switch
            label={i18n._("ai.pluginEnabled")}
            value={draft.pluginEnabled}
            onChange={(pluginEnabled) =>
              setDraft({
                ...draft,
                pluginEnabled,
                summaryEnabled:
                  pluginEnabled && !draft.summaryEnabled ? true : draft.summaryEnabled,
              })
            }
            labelSpacing="spread"
            isDisabled={pluginToggleDisabled}
          />
        </section>
        <section className="ai-operation-section" aria-labelledby="ai-summary-heading">
          <div>
            <div id="ai-summary-heading" className="reader-preference-label">
              {i18n._("ai.summaryTitle")}
            </div>
            <div className="reader-preference-description">
              {i18n._("ai.summaryDescription")}
            </div>
          </div>
          <Selector
            label={i18n._("ai.summaryProvider")}
            value={draft.summaryProviderId || undefined}
            options={providerOptions}
            onChange={(summaryProviderId) =>
              setDraft({ ...draft, summaryProviderId })
            }
            placeholder={i18n._("ai.selectProvider")}
            isDisabled={operationsDisabled}
            width="100%"
          />
          <SegmentedControl
            label={i18n._("ai.summaryStyle")}
            value={draft.summaryStyle}
            onChange={(summaryStyle) =>
              setDraft({ ...draft, summaryStyle: summaryStyle as AiSummaryStyle })
            }
            layout="fill"
            isDisabled={operationsDisabled}
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
            isDisabled={operationsDisabled}
            width="100%"
          />
        </section>
        <div className="reader-dialog-actions">
          <Button
            label={i18n._("ai.configSave")}
            type="submit"
            variant="primary"
            isLoading={props.isSaving}
            isDisabled={!canSubmit}
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
    pluginEnabled: config?.isEnabled ?? false,
    summaryEnabled: config?.summary.enabled ?? false,
    summaryProviderId: config?.summary.providerId ?? defaultProviderId,
    summaryStyle: config?.summary.style ?? "BALANCED",
    summaryMaxOutputTokens: config?.summary.maxOutputTokens ?? 1024,
  }
}
