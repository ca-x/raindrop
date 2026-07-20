import { Heading } from "@astryxdesign/core/Heading"
import { List, ListItem } from "@astryxdesign/core/List"
import { Stack } from "@astryxdesign/core/Stack"
import { Text } from "@astryxdesign/core/Text"
import { useLingui } from "@lingui/react"

import type { AiSummaryArtifact } from "../api/content.generated"

interface SummaryViewProps {
  artifact: AiSummaryArtifact
}

export function SummaryView({ artifact }: SummaryViewProps) {
  const { i18n } = useLingui()
  return (
    <Stack gap={4} className="ai-artifact-view">
      <div className="ai-artifact-meta">
        <Text type="supporting" color="secondary">
          {i18n._("ai.reader.generatedBy", { provider: artifact.providerLabel })}
        </Text>
      </div>
      <section aria-labelledby="ai-summary-result-heading">
        <Heading level={3} id="ai-summary-result-heading">
          {i18n._("ai.reader.summaryResult")}
        </Heading>
        <Text as="p" display="block" textWrap="pretty">
          {artifact.summary}
        </Text>
      </section>
      {artifact.bullets.length > 0 ? (
        <section aria-labelledby="ai-summary-points-heading">
          <Heading level={3} id="ai-summary-points-heading">
            {i18n._("ai.reader.keyPoints")}
          </Heading>
          <List listStyle="disc" density="balanced">
            {artifact.bullets.map((bullet, index) => (
              <ListItem key={`${index}:${bullet}`} label={bullet} />
            ))}
          </List>
        </section>
      ) : null}
      {artifact.conclusion ? (
        <section aria-labelledby="ai-summary-conclusion-heading">
          <Heading level={3} id="ai-summary-conclusion-heading">
            {i18n._("ai.reader.conclusion")}
          </Heading>
          <Text as="p" display="block" textWrap="pretty">
            {artifact.conclusion}
          </Text>
        </section>
      ) : null}
    </Stack>
  )
}
