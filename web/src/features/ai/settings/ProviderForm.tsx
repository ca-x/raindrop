import { Button } from "@astryxdesign/core/Button"
import { CheckboxInput } from "@astryxdesign/core/CheckboxInput"
import { Collapsible } from "@astryxdesign/core/Collapsible"
import { NumberInput } from "@astryxdesign/core/NumberInput"
import { Selector } from "@astryxdesign/core/Selector"
import { Stack } from "@astryxdesign/core/Stack"
import { TextInput } from "@astryxdesign/core/TextInput"
import { useLingui } from "@lingui/react"
import { useState, type FormEvent } from "react"

import type { ProviderKind, ProviderPolicy } from "../api/provider.generated"
import {
  changeProviderKind,
  providerDraftRequest,
  type ProviderDraft,
  type ProviderDraftErrors,
  type ProviderDraftField,
} from "../model/providerDraft"
import { providerKindLabel } from "./ProviderList"

interface ProviderFormProps {
  draft: ProviderDraft
  isSaving: boolean
  credentialAvailable: boolean
  onChange: (draft: ProviderDraft) => void
  onSave: (draft: ProviderDraft) => Promise<boolean>
  onCancel: () => void
}

const PROVIDER_KINDS: ProviderKind[] = [
  "ANTHROPIC_MESSAGES",
  "OPENAI_RESPONSES",
  "OPENAI_CHAT_COMPLETIONS",
  "GOOGLE_GEMINI",
]

export function ProviderForm(props: ProviderFormProps) {
  const { i18n } = useLingui()
  const [errors, setErrors] = useState<ProviderDraftErrors>({})
  const update = (patch: Partial<ProviderDraft>) => {
    props.onChange({ ...props.draft, ...patch })
    setErrors({})
  }
  const updatePolicy = (field: keyof ProviderPolicy, value: number | null) => {
    update({ policy: { ...props.draft.policy, [field]: value } })
  }
  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    const parsed = providerDraftRequest(props.draft)
    if (!parsed.ok) {
      setErrors(parsed.errors)
      return
    }
    await props.onSave(props.draft)
  }

  return (
    <form onSubmit={submit} className="ai-provider-form">
      <Stack gap={4}>
        <TextInput
          label={i18n._("ai.providerName")}
          value={props.draft.displayName}
          onChange={(displayName) => update({ displayName })}
          isRequired
          isDisabled={props.isSaving}
          width="100%"
          status={fieldStatus((id) => i18n._(id), errors, "displayName")}
        />
        <Selector
          label={i18n._("ai.providerKind")}
          value={props.draft.kind}
          options={PROVIDER_KINDS.map((kind) => ({
            value: kind,
            label: providerKindLabel((id) => i18n._(id), kind),
          }))}
          onChange={(kind) =>
            props.onChange(changeProviderKind(props.draft, kind as ProviderKind))
          }
          isDisabled={props.draft.mode === "edit" || props.isSaving}
          disabledMessage={
            props.draft.mode === "edit" ? i18n._("ai.providerKindImmutable") : undefined
          }
          width="100%"
        />
        <TextInput
          label={i18n._("ai.providerEndpoint")}
          description={i18n._("ai.providerEndpointDescription")}
          value={props.draft.endpoint}
          onChange={(endpoint) => update({ endpoint })}
          isRequired
          isDisabled={props.isSaving}
          width="100%"
          status={fieldStatus((id) => i18n._(id), errors, "endpoint")}
        />
        <TextInput
          label={i18n._("ai.providerModel")}
          value={props.draft.model}
          onChange={(model) => update({ model })}
          isRequired
          isDisabled={props.isSaving}
          width="100%"
          status={fieldStatus((id) => i18n._(id), errors, "model")}
        />
        <TextInput
          type="password"
          label={i18n._("ai.providerCredential")}
          description={i18n._(
            props.draft.mode === "edit"
              ? "ai.providerCredentialEditDescription"
              : "ai.providerCredentialCreateDescription",
          )}
          value={props.draft.credential}
          onChange={(credential) => update({ credential })}
          isRequired={props.draft.mode === "create"}
          isOptional={props.draft.mode === "edit"}
          isDisabled={props.isSaving || !props.credentialAvailable}
          disabledMessage={
            !props.credentialAvailable
              ? i18n._("ai.providerCredentialUnavailable")
              : undefined
          }
          width="100%"
          status={fieldStatus((id) => i18n._(id), errors, "credential")}
        />
        <CheckboxInput
          label={i18n._("ai.providerEnabled")}
          description={i18n._("ai.providerEnabledDescription")}
          value={props.draft.isEnabled}
          onChange={(isEnabled) => update({ isEnabled })}
          isDisabled={props.isSaving}
        />
        <Collapsible
          trigger={i18n._("ai.providerAdvanced")}
          defaultIsOpen={false}
        >
          <Stack gap={4} className="ai-provider-advanced">
            <div className="ai-provider-capabilities">
              <CheckboxInput
                label={i18n._("ai.providerSupportsUsage")}
                value={props.draft.capabilities.supportsUsage}
                onChange={(supportsUsage) =>
                  update({
                    capabilities: {
                      ...props.draft.capabilities,
                      supportsUsage,
                    },
                  })
                }
                isDisabled={props.isSaving}
              />
              <CheckboxInput
                label={i18n._("ai.providerSupportsIdempotency")}
                value={props.draft.capabilities.supportsIdempotency}
                onChange={(supportsIdempotency) =>
                  update({
                    capabilities: {
                      ...props.draft.capabilities,
                      supportsIdempotency,
                    },
                  })
                }
                isDisabled={props.isSaving}
              />
            </div>
            <div className="ai-settings-number-grid">
              <PolicyNumber
                field="maxConcurrency"
                label={i18n._("ai.policyConcurrency")}
                value={props.draft.policy.maxConcurrency}
                min={1}
                max={64}
                errors={errors}
                onChange={updatePolicy}
                isDisabled={props.isSaving}
              />
              <PolicyNumber
                field="requestsPerMinute"
                label={i18n._("ai.policyRequestsPerMinute")}
                value={props.draft.policy.requestsPerMinute}
                min={1}
                max={1_000_000}
                hasClear
                errors={errors}
                onChange={updatePolicy}
                isDisabled={props.isSaving}
              />
              <PolicyNumber
                field="maxInputTokensPerRequest"
                label={i18n._("ai.policyInputTokens")}
                value={props.draft.policy.maxInputTokensPerRequest}
                min={1}
                max={1_048_576}
                errors={errors}
                onChange={updatePolicy}
                isDisabled={props.isSaving}
              />
              <PolicyNumber
                field="maxOutputTokensPerRequest"
                label={i18n._("ai.policyOutputTokens")}
                value={props.draft.policy.maxOutputTokensPerRequest}
                min={1}
                max={16_384}
                errors={errors}
                onChange={updatePolicy}
                isDisabled={props.isSaving}
              />
              <PolicyNumber
                field="inputCostMicrosPerMillionTokens"
                label={i18n._("ai.policyInputCost")}
                value={props.draft.policy.inputCostMicrosPerMillionTokens}
                min={0}
                max={1_000_000_000_000}
                hasClear
                errors={errors}
                onChange={updatePolicy}
                isDisabled={props.isSaving}
              />
              <PolicyNumber
                field="outputCostMicrosPerMillionTokens"
                label={i18n._("ai.policyOutputCost")}
                value={props.draft.policy.outputCostMicrosPerMillionTokens}
                min={0}
                max={1_000_000_000_000}
                hasClear
                errors={errors}
                onChange={updatePolicy}
                isDisabled={props.isSaving}
              />
              <PolicyNumber
                field="maxCostMicrosPerRequest"
                label={i18n._("ai.policyRequestCost")}
                value={props.draft.policy.maxCostMicrosPerRequest}
                min={0}
                max={1_000_000_000_000}
                hasClear
                errors={errors}
                onChange={updatePolicy}
                isDisabled={props.isSaving}
              />
            </div>
          </Stack>
        </Collapsible>
        <div className="reader-dialog-actions">
          <Button
            label={i18n._("common.cancel")}
            onClick={props.onCancel}
            isDisabled={props.isSaving}
            variant="secondary"
          />
          <Button
            label={i18n._(
              props.draft.mode === "create"
                ? "ai.providerCreate"
                : "ai.providerSave",
            )}
            type="submit"
            isLoading={props.isSaving}
            variant="primary"
          />
        </div>
      </Stack>
    </form>
  )
}

interface PolicyNumberProps {
  field: keyof ProviderPolicy
  label: string
  value: number | null
  min: number
  max: number
  hasClear?: boolean
  errors: ProviderDraftErrors
  onChange: (field: keyof ProviderPolicy, value: number | null) => void
  isDisabled: boolean
}

function PolicyNumber(props: PolicyNumberProps) {
  const { i18n } = useLingui()
  const shared = {
    label: props.label,
    value: props.value,
    min: props.min,
    max: props.max,
    isIntegerOnly: true,
    isDisabled: props.isDisabled,
    width: "100%" as const,
    status: fieldStatus((id) => i18n._(id), props.errors, props.field),
  }
  return props.hasClear ? (
    <NumberInput
      {...shared}
      hasClear
      onChange={(value) => props.onChange(props.field, value)}
    />
  ) : (
    <NumberInput
      {...shared}
      onChange={(value) => props.onChange(props.field, value)}
    />
  )
}

function fieldStatus(
  translate: (id: string) => string,
  errors: ProviderDraftErrors,
  field: ProviderDraftField,
) {
  const error = errors[field]
  if (!error) return undefined
  const messages = {
    REQUIRED: "ai.fieldRequired",
    TOO_LONG: "ai.fieldTooLong",
    HTTPS: "ai.endpointInvalid",
    RANGE: "ai.numberOutOfRange",
  } as const
  return { type: "error" as const, message: translate(messages[error]) }
}
