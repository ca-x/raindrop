import { Banner } from "@astryxdesign/core/Banner"
import { Button } from "@astryxdesign/core/Button"
import { List, ListItem } from "@astryxdesign/core/List"
import { Popover } from "@astryxdesign/core/Popover"
import { Spinner } from "@astryxdesign/core/Spinner"
import { Stack } from "@astryxdesign/core/Stack"
import { Text } from "@astryxdesign/core/Text"
import { TextInput } from "@astryxdesign/core/TextInput"
import { Toolbar } from "@astryxdesign/core/Toolbar"
import { useLingui } from "@lingui/react"
import { useEffect, useState } from "react"

import type {
  TranslationConfig,
  TranslationDisplayMode,
} from "../api/translation.generated"
import type { EntryTranslationController } from "../model/useEntryTranslationController"
import { displayModeOptions } from "../settings/TranslationSettingsPanel"

interface Props {
  controller: EntryTranslationController
  config: TranslationConfig
  selectedText: string
  onDisplayModeChange: (mode: TranslationDisplayMode) => Promise<boolean>
}

export function TranslationReaderControls(props: Props) {
  const { i18n } = useLingui()
  const [isModeOpen, setIsModeOpen] = useState(false)
  const [isLookupOpen, setIsLookupOpen] = useState(false)
  const [lookupText, setLookupText] = useState("")
  useEffect(() => {
    if (props.selectedText) setLookupText(props.selectedText)
  }, [props.selectedText])
  const result = props.controller.result
  const currentModeLabel = i18n._(
    `translation.displayMode.${props.config.displayMode}`,
  )
  return (
    <div className="reader-translation-controls">
      <Toolbar
        size="md"
        label={i18n._("translation.reader.controls")}
        startContent={
          <div className="reader-translation-status">
            {props.controller.isTranslating ? (
              <Spinner label={i18n._("translation.reader.translating")} size="sm" />
            ) : result ? (
              <Text type="supporting" color="secondary">
                {i18n._("translation.reader.translatedBy", {
                  provider: result.providerLabel,
                })}
              </Text>
            ) : (
              <Text type="supporting" color="secondary">
                {i18n._("translation.reader.ready")}
              </Text>
            )}
          </div>
        }
        endContent={
          <>
            <Button
              label={i18n._(
                result
                  ? "translation.reader.retranslate"
                  : "translation.reader.translateArticle",
              )}
              clickAction={async () => {
                await props.controller.translate()
              }}
              isLoading={props.controller.isTranslating}
              isDisabled={props.controller.isLookingUp}
              variant={result ? "secondary" : "primary"}
            />
            {result ? (
              <Popover
                isOpen={isModeOpen}
                onOpenChange={setIsModeOpen}
                placement="below"
                alignment="end"
                width="min(280px, calc(100vw - 24px))"
                label={i18n._("translation.reader.displayMode")}
                content={
                  <div className="reader-translation-mode-list">
                    {displayModeOptions((id) => i18n._(id)).map((option) => (
                      <Button
                        key={option.value}
                        label={option.label}
                        onClick={() => {
                          void props
                            .onDisplayModeChange(option.value)
                            .then((saved) => saved && setIsModeOpen(false))
                        }}
                        variant={
                          props.config.displayMode === option.value
                            ? "secondary"
                            : "ghost"
                        }
                      />
                    ))}
                  </div>
                }
              >
                <Button
                  label={i18n._("translation.reader.displayModeValue", {
                    mode: currentModeLabel,
                  })}
                  variant="secondary"
                />
              </Popover>
            ) : null}
            <Popover
              isOpen={isLookupOpen}
              onOpenChange={(isOpen) => {
                setIsLookupOpen(isOpen)
                if (!isOpen) props.controller.clearLookup()
              }}
              placement="below"
              alignment="end"
              width="min(380px, calc(100vw - 24px))"
              label={i18n._("translation.reader.lookup")}
              content={
                <Stack gap={3} className="reader-translation-lookup">
                  <TextInput
                    label={i18n._("translation.reader.lookupText")}
                    value={lookupText}
                    description={i18n._("translation.reader.lookupLimit")}
                    onChange={(value) => setLookupText(value.slice(0, 200))}
                    isDisabled={props.controller.isLookingUp}
                    width="100%"
                  />
                  <Button
                    label={i18n._("translation.reader.lookupAction")}
                    clickAction={async () => {
                      await props.controller.lookup(lookupText)
                    }}
                    isLoading={props.controller.isLookingUp}
                    isDisabled={!lookupText.trim() || lookupText.trim().length > 200}
                    variant="primary"
                  />
                  {props.controller.lookupResult ? (
                    <LookupResultView result={props.controller.lookupResult} />
                  ) : null}
                </Stack>
              }
            >
              <Button
                label={i18n._("translation.reader.lookup")}
                variant="secondary"
              />
            </Popover>
            {result ? (
              <Button
                label={i18n._("translation.reader.clear")}
                onClick={props.controller.clearTranslation}
                variant="ghost"
              />
            ) : null}
          </>
        }
      />
      {props.controller.error ? (
        <Banner
          status="error"
          title={i18n._("translation.reader.error")}
          description={i18n._(`translation.reader.error.${props.controller.error}`)}
        />
      ) : null}
    </div>
  )
}

function LookupResultView({
  result,
}: {
  result: NonNullable<EntryTranslationController["lookupResult"]>
}) {
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
