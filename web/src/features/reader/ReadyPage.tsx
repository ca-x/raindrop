import { AppShell } from "@astryxdesign/core/AppShell"
import { EmptyState } from "@astryxdesign/core/EmptyState"
import { useLingui } from "@lingui/react"

export function ReadyPage() {
  const { i18n } = useLingui()
  return (
    <AppShell contentPadding={4} height="fill" variant="section">
      <EmptyState
        headingLevel={1}
        title={i18n._("ready.title")}
        description={i18n._("ready.description")}
      />
    </AppShell>
  )
}
