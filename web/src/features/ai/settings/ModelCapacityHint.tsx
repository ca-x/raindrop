import { Button } from "@astryxdesign/core/Button"
import { Text } from "@astryxdesign/core/Text"
import { useLingui } from "@lingui/react"
import { useEffect, useRef, useState } from "react"

import { lookupModelLimits } from "../api/providers"
import type { ModelLimits, ProviderPolicy } from "../api/provider.generated"
import { mergeModelLimits, recommendedModelBudget } from "../model/modelLimits"

interface Props {
  csrfToken: string
  model: string
  endpoint: string
  limits?: ModelLimits
  isDisabled: boolean
  onApply: (budget: Pick<ProviderPolicy, "maxInputTokensPerRequest" | "maxOutputTokensPerRequest">) => void
}

export function ModelCapacityHint(props: Props) {
  return <CapacityLookup key={`${props.model}:${props.endpoint}:${props.csrfToken}`} {...props} />
}

function CapacityLookup(props: Props) {
  const { i18n } = useLingui()
  const [catalogLimits, setCatalogLimits] = useState<ModelLimits | null>(null)
  const [status, setStatus] = useState<"idle" | "loading" | "unknown" | "unavailable">("idle")
  const active = useRef<AbortController | null>(null)
  useEffect(() => () => active.current?.abort(), [])
  const limits = mergeModelLimits(props.limits, catalogLimits)
  const budget = limits ? recommendedModelBudget(limits) : null

  async function lookup() {
    active.current?.abort()
    const abort = new AbortController()
    active.current = abort
    setStatus("loading")
    try {
      const result = await lookupModelLimits(props.csrfToken, props.model.trim(), props.endpoint, abort.signal)
      if (abort.signal.aborted) return
      setCatalogLimits(result.limits)
      setStatus(result.limits ? "idle" : result.catalogAvailable ? "unknown" : "unavailable")
    } catch {
      if (!abort.signal.aborted) setStatus("unavailable")
    } finally {
      if (active.current === abort) active.current = null
    }
  }

  return <div className="ai-model-capacity">
    {limits ? <>
      <dl className="ai-model-capacity-values">
        {([
          ["ai.modelContext", limits.contextWindow],
          ["ai.modelInput", limits.maxInputTokens],
          ["ai.modelOutput", limits.maxOutputTokens],
        ] as const).map(([label, value]) => <div key={label}>
          <dt>{i18n._(label)}</dt><dd>{value?.toLocaleString(i18n.locale) ?? i18n._("ai.modelCapacityUnknown")}</dd>
        </div>)}
      </dl>
      <Text as="p" display="block" type="supporting" color="secondary">{i18n._(`ai.modelSource.${limits.source}`)}</Text>
      {limits.matchKind === "ALIAS" && limits.matchedModel ? <Text as="p" display="block" type="supporting" color="secondary">
        {i18n._("ai.modelAliasMatch", { model: limits.matchedModel })}
      </Text> : null}
    </> : null}
    <div className="ai-model-capacity-actions">
      <Button type="button" label={i18n._("ai.modelCapacityLookup")} variant="ghost" isLoading={status === "loading"}
        isDisabled={props.isDisabled || !props.model.trim() || status === "loading"} onClick={() => void lookup()} />
      {budget ? <Button type="button" label={i18n._("ai.modelCapacityApply")} variant="secondary" isDisabled={props.isDisabled}
        onClick={() => props.onApply(budget)} /> : null}
    </div>
    <Text as="p" display="block" type="supporting" color="secondary" role="status">
      {i18n._(status === "unknown" ? "ai.modelCapacityNotFound" : status === "unavailable" ? "ai.modelCapacityUnavailable" : "ai.modelCapacityHelp")}
    </Text>
  </div>
}
