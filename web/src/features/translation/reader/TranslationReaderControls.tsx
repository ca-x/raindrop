import { Banner } from "@astryxdesign/core/Banner"
import { Button } from "@astryxdesign/core/Button"
import { Popover } from "@astryxdesign/core/Popover"
import { Spinner } from "@astryxdesign/core/Spinner"
import { Text } from "@astryxdesign/core/Text"
import { Toolbar } from "@astryxdesign/core/Toolbar"
import { useLingui } from "@lingui/react"
import { useState } from "react"

import type {
  TranslationConfig,
  TranslationDisplayMode,
} from "../api/translation.generated"
import type { EntryTranslationController } from "../model/useEntryTranslationController"
import { displayModeOptions } from "../settings/TranslationSettingsPanel"

interface Props {
  controller: EntryTranslationController
  config: TranslationConfig
  onDisplayModeChange: (mode: TranslationDisplayMode) => Promise<boolean>
}

export function TranslationReaderControls(props: Props) {
  const { i18n } = useLingui()
  const [isModeOpen, setIsModeOpen] = useState(false)
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
              <Spinner
                label={i18n._(
                  props.controller.totalSegments > 0
                    ? "translation.reader.translatingProgress"
                    : "translation.reader.translating",
                  {
                    completed: props.controller.completedSegments,
                    total: props.controller.totalSegments,
                  },
                )}
                size="sm"
              />
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
              onClick={() => void props.controller.translate()}
              isLoading={props.controller.isTranslating}
              isDisabled={
                props.controller.isLookingUp ||
                props.controller.isTranslatingSelection
              }
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
      {props.controller.articleError ? (
        <Banner
          status="error"
          title={i18n._("translation.reader.error")}
          description={i18n._(
            `translation.reader.error.${props.controller.articleError}`,
          )}
        />
      ) : null}
    </div>
  )
}
