import type { ModelLimits } from "../api/provider.generated"

export function mergeModelLimits(service: ModelLimits | undefined, catalog: ModelLimits | null): ModelLimits | null {
  if (!service || !catalog) return service ?? catalog
  const minimum = (left: number | null, right: number | null) => left === null ? right : right === null ? left : Math.min(left, right)
  return {
    contextWindow: minimum(service.contextWindow, catalog.contextWindow),
    maxInputTokens: minimum(service.maxInputTokens, catalog.maxInputTokens),
    maxOutputTokens: minimum(service.maxOutputTokens, catalog.maxOutputTokens),
    source: "COMBINED",
    ...((service.matchedModel ?? catalog.matchedModel) ? { matchedModel: service.matchedModel ?? catalog.matchedModel } : {}),
    ...((service.matchKind ?? catalog.matchKind) ? { matchKind: service.matchKind ?? catalog.matchKind } : {}),
  }
}

function aliasKey(model: string) {
  const leaf = model.trim().split("/").at(-1) ?? ""
  return /^[\x00-\x7f]+$/u.test(leaf) ? leaf.replace(/[._:-]/gu, "").toLowerCase() : null
}

/** A draft must not lose a known smaller service limit through whitespace or gateway aliases. */
export function discoveredModelLimits(models: { id: string; limits?: ModelLimits }[], model: string): ModelLimits | undefined {
  const exact = models.find((candidate) => candidate.id === model.trim())
  if (exact) return exact.limits
  const key = aliasKey(model)
  if (!key) return undefined
  const aliases = models.filter((candidate) => aliasKey(candidate.id) === key && candidate.limits)
  const first = aliases[0]
  if (!first?.limits) return undefined
  let limits = first.limits
  for (const alias of aliases.slice(1)) {
    limits = mergeModelLimits(limits, alias.limits ?? null) ?? limits
  }
  return { ...limits, matchedModel: first.id, matchKind: "ALIAS" }
}

/** Reserve input space inside the shared context window; settings remain user-editable. */
export function recommendedModelBudget(limits: ModelLimits) {
  const context = limits.contextWindow
  const output = limits.maxOutputTokens
  if (!output) return null
  const maxOutputTokensPerRequest = Math.min(output, context ? Math.floor(context / 2) : output, 1_048_576)
  const input = limits.maxInputTokens ?? context
  if (!input || maxOutputTokensPerRequest < 1) return null
  const maxInputTokensPerRequest = Math.min(input, context ? context - maxOutputTokensPerRequest : input, 1_048_576)
  if (maxInputTokensPerRequest < 1) return null
  return { maxInputTokensPerRequest, maxOutputTokensPerRequest }
}
