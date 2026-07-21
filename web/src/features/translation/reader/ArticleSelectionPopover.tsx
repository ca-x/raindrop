import { Button } from "@astryxdesign/core/Button"
import { Popover } from "@astryxdesign/core/Popover"
import { Spinner } from "@astryxdesign/core/Spinner"
import { Text } from "@astryxdesign/core/Text"
import { useLingui } from "@lingui/react"
import {
  type ReactNode,
  type RefObject,
  useCallback,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react"

import type { EntryTranslationController } from "../model/useEntryTranslationController"
import {
  MAX_LOOKUP_SELECTION_CHARACTERS,
  MAX_TRANSLATION_SELECTION_CHARACTERS,
  readSelectedArticleText,
  selectionExcerpt,
  unicodeLength,
} from "./articleSelection"
import {
  LookupResultView,
  SelectionTranslationResultView,
} from "./TranslationResultViews"

type SelectionAction = "LOOKUP" | "TRANSLATE"

interface Props {
  children: ReactNode
  controller: EntryTranslationController
  isEnabled: boolean
}

export function ArticleSelectionPopover({
  children,
  controller,
  isEnabled,
}: Props) {
  const { i18n } = useLingui()
  const containerRef = useRef<HTMLDivElement>(null)
  const anchorRef = useRef<HTMLButtonElement>(null)
  const [isOpen, setIsOpen] = useState(false)
  const [activeText, setActiveText] = useState("")
  const [action, setAction] = useState<SelectionAction>("LOOKUP")
  const previousEntryIdRef = useRef(controller.entryId)
  const activeLength = unicodeLength(activeText)
  const excerpt = useMemo(() => selectionExcerpt(activeText), [activeText])

  useLayoutEffect(() => {
    if (previousEntryIdRef.current === controller.entryId) return
    previousEntryIdRef.current = controller.entryId
    setIsOpen(false)
    setActiveText("")
    controller.cancelContextActions()
  }, [controller.cancelContextActions, controller.entryId])

  const runAction = useCallback(
    (nextAction: SelectionAction, text: string) => {
      setAction(nextAction)
      if (nextAction === "LOOKUP") {
        void controller.lookup(text)
      } else {
        void controller.translateSelection(text)
      }
    },
    [controller],
  )

  return (
    <div
      ref={containerRef}
      className="reader-selection-trigger"
      onContextMenu={(event) => {
        const selectedText = readSelectedArticleText(containerRef.current)
        const selectedLength = unicodeLength(selectedText)
        const canOpen =
          isEnabled &&
          controller.entryId !== null &&
          !controller.isTranslating &&
          selectedLength > 0 &&
          selectedLength <= MAX_TRANSLATION_SELECTION_CHARACTERS
        if (!canOpen) return
        event.preventDefault()
        const container = containerRef.current
        const anchor = anchorRef.current
        const rect = container?.getBoundingClientRect()
        if (anchor && rect) {
          anchor.style.left = `${event.clientX - rect.left}px`
          anchor.style.top = `${event.clientY - rect.top}px`
        }
        const defaultAction =
          selectedLength <= MAX_LOOKUP_SELECTION_CHARACTERS
            ? "LOOKUP"
            : "TRANSLATE"
        controller.cancelContextActions()
        setActiveText(selectedText)
        setIsOpen(true)
        runAction(defaultAction, selectedText)
      }}
    >
      {children}
      <button
        ref={anchorRef}
        type="button"
        tabIndex={-1}
        aria-hidden="true"
        className="reader-selection-anchor"
      />
      <Popover
        anchorRef={anchorRef as RefObject<HTMLElement>}
        isOpen={isOpen}
        onOpenChange={(nextOpen) => {
          setIsOpen(nextOpen)
          if (!nextOpen) {
            setActiveText("")
            controller.cancelContextActions()
          }
        }}
        placement="below"
        alignment="start"
        width="min(390px, calc(100vw - 24px))"
        label={i18n._("translation.reader.selectionPopover")}
        closeButtonLabel={i18n._("translation.reader.closeSelectionPopover")}
        content={
          <div className="reader-selection-popover">
            <div className="reader-selection-popover-header">
              <Text type="supporting" color="secondary">
                {i18n._(
                  action === "LOOKUP"
                    ? "translation.reader.lookupSelection"
                    : "translation.reader.translateSelection",
                )}
              </Text>
              <Text as="p" type="supporting">
                {excerpt}
              </Text>
            </div>
            {activeLength > 0 &&
            activeLength <= MAX_LOOKUP_SELECTION_CHARACTERS ? (
              <div
                className="reader-selection-mode-switch"
                role="group"
                aria-label={i18n._("translation.reader.selectionMode")}
              >
                <Button
                  label={i18n._("translation.reader.lookupSelection")}
                  onClick={() => runAction("LOOKUP", activeText)}
                  isDisabled={controller.isTranslating}
                  aria-pressed={action === "LOOKUP"}
                  variant={action === "LOOKUP" ? "secondary" : "ghost"}
                />
                <Button
                  label={i18n._("translation.reader.translateSelection")}
                  onClick={() => runAction("TRANSLATE", activeText)}
                  isDisabled={controller.isTranslating}
                  aria-pressed={action === "TRANSLATE"}
                  variant={action === "TRANSLATE" ? "secondary" : "ghost"}
                />
              </div>
            ) : null}
            <div className="reader-selection-popover-result">
              {controller.isLookingUp || controller.isTranslatingSelection ? (
                <div className="reader-selection-loading" role="status">
                  <Spinner
                    label={i18n._(
                      controller.isLookingUp
                        ? "translation.reader.lookingUpSelection"
                        : "translation.reader.translatingSelection",
                    )}
                    size="sm"
                  />
                </div>
              ) : null}
              {controller.lookupResult ? (
                <LookupResultView result={controller.lookupResult} />
              ) : null}
              {controller.selectionResult ? (
                <SelectionTranslationResultView
                  result={controller.selectionResult}
                />
              ) : null}
              {controller.contextError ? (
                <div className="reader-selection-error" role="alert">
                  {i18n._(`translation.reader.error.${controller.contextError}`)}
                </div>
              ) : null}
            </div>
          </div>
        }
      />
    </div>
  )
}
