import { useHotkeys } from "@astryxdesign/core/hooks"
import { useEffect, useRef } from "react"

export interface UseReaderHotkeysOptions {
  queueEntryIds: string[]
  cursorEntryId: string | null
  openEntryId: string | null
  isDisabled: boolean
  isQueueDisabled?: boolean
  isUnread: (entryId: string) => boolean
  onCursorChange: (entryId: string) => void
  onOpenEntry: (entryId: string) => void
  onToggleRead: (entryId: string) => void | Promise<void>
  onToggleStar: (entryId: string) => void | Promise<void>
  onNextUnreadSource: () => void | Promise<void>
  onPreviousUnreadSource: () => void | Promise<void>
  canToggleImmersive?: boolean
  isImmersive?: boolean
  onToggleImmersive?: () => void
  onExitImmersive?: () => void
  onFocusSources?: () => void
  onFocusQueue?: () => void
  canScrollArticle?: boolean
  onScrollArticle?: (direction: 1 | -1) => void
}

const editableSelector = [
  "[role='textbox']",
  "[role='searchbox']",
  "[role='combobox']",
  "[role='spinbutton']",
  "[role='slider']",
].join(",")
const ariaModalSelector = [
  ":not(dialog)[role='dialog'][aria-modal='true']",
  ":not(dialog)[role='alertdialog'][aria-modal='true']",
].join(",")
const readerKeys = new Set([
  "j",
  "k",
  "n",
  "p",
  "m",
  "s",
  "o",
  "f",
  "escape",
  "[",
  "]",
  " ",
])

export function useReaderHotkeys(options: UseReaderHotkeysOptions): void {
  const optionsRef = useRef(options)
  optionsRef.current = options
  const disabledRef = useRef(options.isDisabled)
  disabledRef.current = options.isDisabled
  const queueDisabledRef = useRef(Boolean(options.isQueueDisabled))
  queueDisabledRef.current = Boolean(options.isQueueDisabled)
  useImmediateInteractionGuard(optionsRef, disabledRef)
  const move = (direction: 1 | -1, open: boolean) => {
    const current = optionsRef.current
    const target = adjacentEntry(current.queueEntryIds, current.cursorEntryId, direction)
    if (!target) return
    current.onCursorChange(target)
    if (!open) return
    current.onOpenEntry(target)
    if (current.isUnread(target)) void current.onToggleRead(target)
  }
  const toggle = (field: "read" | "star", event: KeyboardEvent) => {
    if (event.repeat) return
    const current = optionsRef.current
    const target = current.cursorEntryId ?? current.openEntryId
    if (!target) return
    if (field === "read") void current.onToggleRead(target)
    else void current.onToggleStar(target)
  }
  const openCurrent = (event: KeyboardEvent) => {
    if (event.repeat) return
    const current = optionsRef.current
    const target = current.cursorEntryId ?? current.openEntryId
    if (!target) return
    current.onOpenEntry(target)
    if (current.isUnread(target)) void current.onToggleRead(target)
  }
  const guardedHotkey = (
    keys: string,
    onPress: (event: KeyboardEvent) => void,
    isAvailable?: (current: UseReaderHotkeysOptions) => boolean,
    requiresQueue = false,
  ) => ({
    keys,
    onPress,
    get isDisabled() {
      return disabledRef.current ||
        (requiresQueue && queueDisabledRef.current) ||
        (isAvailable ? !isAvailable(optionsRef.current) : false)
    },
  })

  useHotkeys([
    guardedHotkey("shift+j", (event) => {
      if (!event.repeat) void optionsRef.current.onNextUnreadSource()
    }, undefined, true),
    guardedHotkey("shift+k", (event) => {
      if (!event.repeat) void optionsRef.current.onPreviousUnreadSource()
    }, undefined, true),
    guardedHotkey("j", () => move(1, true), undefined, true),
    guardedHotkey("k", () => move(-1, true), undefined, true),
    guardedHotkey("n", () => move(1, false), undefined, true),
    guardedHotkey("p", () => move(-1, false), undefined, true),
    guardedHotkey("m", (event) => toggle("read", event), undefined, true),
    guardedHotkey("s", (event) => toggle("star", event), undefined, true),
    guardedHotkey(
      "o",
      openCurrent,
      (current) => Boolean(current.cursorEntryId ?? current.openEntryId),
      true,
    ),
    guardedHotkey("f", (event) => {
      if (event.repeat) return
      optionsRef.current.onToggleImmersive?.()
    }, (current) => Boolean(current.canToggleImmersive && current.onToggleImmersive)),
    guardedHotkey("escape", (event) => {
      if (event.repeat) return
      optionsRef.current.onExitImmersive?.()
    }, (current) => Boolean(
      current.isImmersive &&
      current.onExitImmersive &&
      !isEditableTarget(document.activeElement)
    )),
    guardedHotkey("[", (event) => {
      if (!event.repeat) optionsRef.current.onFocusSources?.()
    }, (current) => Boolean(current.onFocusSources)),
    guardedHotkey("]", (event) => {
      if (!event.repeat) optionsRef.current.onFocusQueue?.()
    }, (current) => Boolean(current.onFocusQueue)),
    guardedHotkey(
      "shift+space",
      () => optionsRef.current.onScrollArticle?.(-1),
      (current) => Boolean(current.canScrollArticle && current.onScrollArticle),
    ),
    guardedHotkey(
      "space",
      () => optionsRef.current.onScrollArticle?.(1),
      (current) => Boolean(current.canScrollArticle && current.onScrollArticle),
    ),
  ])
}

function adjacentEntry(
  queue: string[],
  cursorEntryId: string | null,
  direction: 1 | -1,
): string | null {
  if (cursorEntryId === null) return direction === 1 ? (queue[0] ?? null) : null
  const currentIndex = queue.indexOf(cursorEntryId)
  if (currentIndex === -1) return direction === 1 ? (queue[0] ?? null) : null
  return queue[currentIndex + direction] ?? null
}

function isAdditionalEditable(target: EventTarget | null): boolean {
  return target instanceof Element && target.closest(editableSelector) !== null
}

function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) return false
  return target.closest("input, textarea, select, [contenteditable]:not([contenteditable='false'])") !== null ||
    isAdditionalEditable(target)
}

function isImeKeyEvent(event: KeyboardEvent): boolean {
  return event.isComposing || event.keyCode === 229
}

function hasOpenModal(): boolean {
  if (document.querySelector("dialog[open]")) return true
  return [...document.querySelectorAll<HTMLElement>(ariaModalSelector)].some(
    (modal) => {
      const popover = modal.closest<HTMLElement>("[popover]")
      return !popover || popover.matches(":popover-open")
    },
  )
}

function useImmediateInteractionGuard(
  optionsRef: { current: UseReaderHotkeysOptions },
  disabledRef: { current: boolean },
): void {
  useEffect(() => {
    const guard = (event: KeyboardEvent) => {
      if (!readerKeys.has(event.key.toLowerCase())) return
      if (event.ctrlKey || event.metaKey || event.altKey) return
      if (hasOpenModal()) {
        event.stopImmediatePropagation()
        return
      }
      if (event.key === "Escape" && isEditableTarget(event.target)) {
        if (isImeKeyEvent(event)) return
        const current = optionsRef.current
        if (disabledRef.current || !current.isImmersive || !current.onExitImmersive) return
        event.preventDefault()
        event.stopImmediatePropagation()
        current.onExitImmersive()
        return
      }
      if (
        isAdditionalEditable(event.target) ||
        (event.key === " " && isNativeSpaceTarget(event.target))
      ) {
        event.stopImmediatePropagation()
      }
    }
    window.addEventListener("keydown", guard, { capture: true })
    return () => window.removeEventListener("keydown", guard, { capture: true })
  }, [])
}

function isNativeSpaceTarget(target: EventTarget | null): boolean {
  return target instanceof Element &&
    target.closest([
      "button",
      "a[href]",
      "input",
      "select",
      "textarea",
      "summary",
      "[role='button']",
      "[role='menuitem']",
      "[role='menuitemcheckbox']",
      "[role='menuitemradio']",
      "[role='option']",
      "[role='treeitem']",
      "[role='checkbox']",
      "[role='radio']",
      "[role='switch']",
      "[role='tab']",
    ].join(",")) !== null
}
