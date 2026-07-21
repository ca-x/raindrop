import { Button } from "@astryxdesign/core/Button"
import { Popover } from "@astryxdesign/core/Popover"
import { Spinner } from "@astryxdesign/core/Spinner"
import { Text } from "@astryxdesign/core/Text"
import { useLingui } from "@lingui/react"
import {
  type ReactNode,
  type RefObject,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react"

import type { EntryTranslationController } from "../model/useEntryTranslationController"
import {
  MAX_LOOKUP_SELECTION_CHARACTERS,
  MAX_TRANSLATION_SELECTION_CHARACTERS,
  type ArticleSelectionAnchor,
  readSelectedArticleAnchor,
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
  const selectionTimerRef = useRef<number | null>(null)
  const [isOpen, setIsOpen] = useState(false)
  const [activeText, setActiveText] = useState("")
  const [action, setAction] = useState<SelectionAction>("LOOKUP")
  const [anchorPosition, setAnchorPosition] = useState({ left: 0, top: 0 })
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

  const clearSelectionTimer = useCallback(() => {
    if (selectionTimerRef.current === null) return
    window.clearTimeout(selectionTimerRef.current)
    selectionTimerRef.current = null
  }, [])

  const syncSelection = useCallback(
    (pointer?: ArticleSelectionAnchor) => {
      const container = containerRef.current
      const selectedText = readSelectedArticleText(container)
      const selectedLength = unicodeLength(selectedText)
      const canOfferAction =
        isEnabled &&
        controller.entryId !== null &&
        !controller.isTranslating &&
        selectedLength > 0 &&
        selectedLength <= MAX_TRANSLATION_SELECTION_CHARACTERS
      if (!container || !canOfferAction) {
        if (!isOpen) setActiveText("")
        return
      }
      const containerRect = container.getBoundingClientRect()
      const selectionAnchor =
        pointer ??
        readSelectedArticleAnchor(container) ?? {
          clientX: containerRect.left + containerRect.width / 2,
          clientY: containerRect.top,
        }
      const horizontalInset = 24
      const maximumLeft = Math.max(
        horizontalInset,
        containerRect.width - horizontalInset,
      )
      setAnchorPosition({
        left: Math.min(
          Math.max(selectionAnchor.clientX - containerRect.left, horizontalInset),
          maximumLeft,
        ),
        top: Math.min(
          Math.max(selectionAnchor.clientY - containerRect.top, 0),
          containerRect.height,
        ),
      })
      const defaultAction =
        selectedLength <= MAX_LOOKUP_SELECTION_CHARACTERS
          ? "LOOKUP"
          : "TRANSLATE"
      controller.cancelContextActions()
      setAction(defaultAction)
      setActiveText(selectedText)
      setIsOpen(false)
    },
    [controller, isEnabled, isOpen],
  )

  useEffect(() => {
    const scheduleSelectionSync = () => {
      clearSelectionTimer()
      selectionTimerRef.current = window.setTimeout(() => {
        selectionTimerRef.current = null
        syncSelection()
      }, 180)
    }
    document.addEventListener("selectionchange", scheduleSelectionSync)
    return () => {
      document.removeEventListener("selectionchange", scheduleSelectionSync)
      clearSelectionTimer()
    }
  }, [clearSelectionTimer, syncSelection])

  useEffect(() => {
    if (isEnabled && controller.entryId !== null && !controller.isTranslating) {
      return
    }
    clearSelectionTimer()
    setIsOpen(false)
    setActiveText("")
    controller.cancelContextActions()
  }, [
    clearSelectionTimer,
    controller.cancelContextActions,
    controller.entryId,
    controller.isTranslating,
    isEnabled,
  ])

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
      onPointerUp={(event) => {
        if ((event.target as Element).closest(".reader-selection-action")) return
        clearSelectionTimer()
        syncSelection({ clientX: event.clientX, clientY: event.clientY })
      }}
    >
      {children}
      <Button
        ref={anchorRef}
        label={i18n._("translation.reader.selectionAction")}
        tooltip={i18n._("translation.reader.selectionAction")}
        icon={<span className="reader-selection-action-glyph">译</span>}
        isIconOnly
        size="sm"
        variant="primary"
        className="reader-selection-action"
        style={{ left: anchorPosition.left, top: anchorPosition.top }}
        data-hidden={activeLength === 0 ? "true" : "false"}
        data-popover-open={isOpen ? "true" : "false"}
        aria-hidden={activeLength === 0 || isOpen}
        tabIndex={activeLength === 0 || isOpen ? -1 : 0}
        onPointerDown={(event) => event.preventDefault()}
        onClick={() => {
          if (activeLength === 0) return
          setIsOpen(true)
          runAction(action, activeText)
        }}
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
        alignment="center"
        width="min(390px, calc(100vw - 48px))"
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
