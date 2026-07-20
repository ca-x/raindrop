import { Badge } from "@astryxdesign/core/Badge"
import { Button } from "@astryxdesign/core/Button"
import { List, ListItem } from "@astryxdesign/core/List"
import { StatusDot } from "@astryxdesign/core/StatusDot"
import { useLingui } from "@lingui/react"

import type { Provider, ProviderKind } from "../api/provider.generated"

interface ProviderListProps {
  providers: Provider[]
  editingProviderId: string | null
  onEdit: (provider: Provider) => void
}

export function ProviderList(props: ProviderListProps) {
  const { i18n } = useLingui()
  if (props.providers.length === 0) {
    return (
      <div className="ai-settings-empty">
        <div className="reader-preference-label">{i18n._("ai.providersEmpty")}</div>
        <div className="reader-preference-description">
          {i18n._("ai.providersEmptyDescription")}
        </div>
      </div>
    )
  }

  return (
    <List density="balanced" hasDividers className="ai-provider-list">
      {props.providers.map((provider) => (
        <ListItem
          key={provider.providerId}
          label={provider.displayName}
          description={`${providerKindLabel((id) => i18n._(id), provider.kind)} · ${provider.model}`}
          startContent={
            <StatusDot
              variant={provider.isEnabled ? "success" : "neutral"}
              label={i18n._(
                provider.isEnabled ? "ai.providerEnabled" : "ai.providerDisabled",
              )}
            />
          }
          endContent={
            <div className="ai-provider-list-actions">
              <Badge
                variant={provider.scope === "INSTANCE" ? "info" : "neutral"}
                label={i18n._(
                  provider.scope === "INSTANCE"
                    ? "ai.providerScopeInstance"
                    : "ai.providerScopeUser",
                )}
              />
              {provider.canEdit ? (
                <Button
                  label={i18n._("ai.providerEdit")}
                  onClick={() => props.onEdit(provider)}
                  variant="secondary"
                  isDisabled={props.editingProviderId === provider.providerId}
                />
              ) : null}
            </div>
          }
        />
      ))}
    </List>
  )
}

export function providerKindLabel(
  translate: (id: string) => string,
  kind: ProviderKind,
): string {
  const labels: Record<ProviderKind, string> = {
    ANTHROPIC_MESSAGES: "ai.providerKindAnthropic",
    OPENAI_RESPONSES: "ai.providerKindOpenAiResponses",
    OPENAI_CHAT_COMPLETIONS: "ai.providerKindOpenAiChat",
    GOOGLE_GEMINI: "ai.providerKindGemini",
  }
  return translate(labels[kind])
}
