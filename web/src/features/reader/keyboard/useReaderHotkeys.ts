import { useHotkeys } from "@astryxdesign/core/hooks"
import { useEffect, useRef } from "react"

export interface UseReaderHotkeysOptions {
  queueEntryIds: string[]
  cursorEntryId: string | null
  openEntryId: string | null
  isDisabled: boolean
  isUnread: (entryId: string) => boolean
  onCursorChange: (entryId: string) => void
  onOpenEntry: (entryId: string) => void
  onToggleRead: (entryId: string) => void | Promise<void>
  onToggleStar: (entryId: string) => void | Promise<void>
  onNextUnreadSource: () => void | Promise<void>
  onPreviousUnreadSource: () => void | Promise<void>
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
const readerKeys = new Set(["j", "k", "n", "p", "m", "s"])

export function useReaderHotkeys(options: UseReaderHotkeysOptions): void {
  useImmediateInteractionGuard()
  const optionsRef = useRef(options)
  optionsRef.current = options
  const disabledRef = useRef(options.isDisabled)
  disabledRef.current = options.isDisabled
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
  const guardedHotkey = (
    keys: string,
    onPress: (event: KeyboardEvent) => void,
  ) => ({
    keys,
    onPress,
    get isDisabled() {
      return disabledRef.current
    },
  })

  useHotkeys([
    guardedHotkey("shift+j", (event) => {
      if (!event.repeat) void optionsRef.current.onNextUnreadSource()
    }),
    guardedHotkey("shift+k", (event) => {
      if (!event.repeat) void optionsRef.current.onPreviousUnreadSource()
    }),
    guardedHotkey("j", () => move(1, true)),
    guardedHotkey("k", () => move(-1, true)),
    guardedHotkey("n", () => move(1, false)),
    guardedHotkey("p", () => move(-1, false)),
    guardedHotkey("m", (event) => toggle("read", event)),
    guardedHotkey("s", (event) => toggle("star", event)),
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

function hasOpenModal(): boolean {
  if (document.querySelector("dialog[open]")) return true
  return [...document.querySelectorAll<HTMLElement>(ariaModalSelector)].some(
    (modal) => {
      const popover = modal.closest<HTMLElement>("[popover]")
      return !popover || popover.matches(":popover-open")
    },
  )
}

function useImmediateInteractionGuard(): void {
  useEffect(() => {
    const guard = (event: KeyboardEvent) => {
      if (!readerKeys.has(event.key.toLowerCase())) return
      if (event.ctrlKey || event.metaKey || event.altKey) return
      if (
        isAdditionalEditable(event.target) ||
        hasOpenModal()
      ) {
        event.stopImmediatePropagation()
      }
    }
    window.addEventListener("keydown", guard, { capture: true })
    return () => window.removeEventListener("keydown", guard, { capture: true })
  }, [])
}
