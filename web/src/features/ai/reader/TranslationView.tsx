import { Heading } from "@astryxdesign/core/Heading"
import { Markdown } from "@astryxdesign/core/Markdown"
import { Stack } from "@astryxdesign/core/Stack"
import { Text } from "@astryxdesign/core/Text"
import { useLingui } from "@lingui/react"
import type { MouseEvent } from "react"

import type { AiTranslationArtifact } from "../api/content.generated"

interface TranslationViewProps {
  artifact: AiTranslationArtifact
}

export function TranslationView({ artifact }: TranslationViewProps) {
  const { i18n } = useLingui()
  return (
    <Stack gap={4} className="ai-artifact-view">
      <div className="ai-artifact-meta">
        <Text type="supporting" color="secondary">
          {i18n._("ai.reader.translationMeta", {
            provider: artifact.providerLabel,
            locale: artifact.targetLocale,
          })}
        </Text>
      </div>
      <Heading level={3}>{artifact.title}</Heading>
      <Markdown
        headingLevelStart={3}
        contentWidth="100%"
        onLinkClick={safeExternalMarkdownLink}
      >
        {artifact.bodyMarkdown}
      </Markdown>
    </Stack>
  )
}

export function safeExternalMarkdownLink(
  href: string,
  _event: MouseEvent<HTMLAnchorElement>,
): void | false {
  try {
    const url = new URL(href)
    if (url.protocol === "http:" || url.protocol === "https:") return
  } catch {
    // Relative and malformed links remain inert.
  }
  return false
}
