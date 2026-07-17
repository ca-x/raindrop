import { useHotkeys } from "@astryxdesign/core/hooks"
import { useEffect, useState } from "react"

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
}

const editableSelector = [
  "[role='textbox']",
  "[role='searchbox']",
  "[role='combobox']",
  "[role='spinbutton']",
  "[role='slider']",
].join(",")
const modalSelector = "dialog[open], [role='dialog'][aria-modal='true'], [role='alertdialog'][aria-modal='true']"
const readerKeys = new Set(["j", "k", "n", "p", "m", "s"])

export function useReaderHotkeys(options: UseReaderHotkeysOptions): void {
  useImmediateModalGuard()
  const hasEditableFocus = useReaderEditableFocus()
  const isDisabled = options.isDisabled || hasEditableFocus
  const move = (direction: 1 | -1, open: boolean) => {
    const target = adjacentEntry(options.queueEntryIds, options.cursorEntryId, direction)
    if (!target) return
    options.onCursorChange(target)
    if (!open) return
    options.onOpenEntry(target)
    if (options.isUnread(target)) void options.onToggleRead(target)
  }
  const toggle = (field: "read" | "star", event: KeyboardEvent) => {
    if (event.repeat) return
    const target = options.cursorEntryId ?? options.openEntryId
    if (!target) return
    if (field === "read") void options.onToggleRead(target)
    else void options.onToggleStar(target)
  }

  useHotkeys([
    { keys: "j", onPress: () => move(1, true), isDisabled },
    { keys: "k", onPress: () => move(-1, true), isDisabled },
    { keys: "n", onPress: () => move(1, false), isDisabled },
    { keys: "p", onPress: () => move(-1, false), isDisabled },
    { keys: "m", onPress: (event) => toggle("read", event), isDisabled },
    { keys: "s", onPress: (event) => toggle("star", event), isDisabled },
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

function useReaderEditableFocus(): boolean {
  const [hasEditableFocus, setHasEditableFocus] = useState(
    () => isAdditionalEditable(document.activeElement),
  )
  useEffect(() => {
    const update = (event: FocusEvent) => {
      const target = event.type === "focusin" ? event.target : event.relatedTarget
      setHasEditableFocus(isAdditionalEditable(target))
    }
    document.addEventListener("focusin", update)
    document.addEventListener("focusout", update)
    return () => {
      document.removeEventListener("focusin", update)
      document.removeEventListener("focusout", update)
    }
  }, [])
  return hasEditableFocus
}

function isAdditionalEditable(target: EventTarget | null): boolean {
  return target instanceof Element && target.closest(editableSelector) !== null
}

function useImmediateModalGuard(): void {
  useEffect(() => {
    const guard = (event: KeyboardEvent) => {
      if (!readerKeys.has(event.key.toLowerCase())) return
      if (event.ctrlKey || event.metaKey || event.altKey) return
      if (document.querySelector(modalSelector)) event.stopImmediatePropagation()
    }
    window.addEventListener("keydown", guard, { capture: true })
    return () => window.removeEventListener("keydown", guard, { capture: true })
  }, [])
}
