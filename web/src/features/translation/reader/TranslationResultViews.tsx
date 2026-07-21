import { List, ListItem } from "@astryxdesign/core/List"
import { Text } from "@astryxdesign/core/Text"
import { useLingui } from "@lingui/react"

import type {
  LookupResult,
  TranslationTextResult,
} from "../api/translation.generated"

export function LookupResultView({ result }: { result: LookupResult }) {
  const { i18n } = useLingui()
  return (
    <div className="reader-lookup-result" aria-live="polite">
      <div className="reader-lookup-query">{result.query}</div>
      <div className="reader-lookup-translation">{result.translation}</div>
      {result.definition ? <p>{result.definition}</p> : null}
      {result.examples.length > 0 ? (
        <List density="compact" hasDividers>
          {result.examples.map((example, index) => (
            <ListItem
              key={`${index}:${example.source}`}
              label={example.source}
              description={example.target}
            />
          ))}
        </List>
      ) : null}
      <Text type="supporting" color="secondary">
        {i18n._("translation.reader.lookupProvider", {
          provider: result.providerLabel,
        })}
      </Text>
    </div>
  )
}

export function SelectionTranslationResultView({
  result,
}: {
  result: TranslationTextResult
}) {
  const { i18n } = useLingui()
  return (
    <div className="reader-selection-translation-result" aria-live="polite">
      <div className="reader-selection-translation-text" lang={result.targetLocale}>
        {result.translatedText}
      </div>
      <Text type="supporting" color="secondary">
        {i18n._("translation.reader.selectionProvider", {
          provider: result.providerLabel,
        })}
      </Text>
    </div>
  )
}
