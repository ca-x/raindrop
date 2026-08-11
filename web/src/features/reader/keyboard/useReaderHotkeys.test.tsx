import { fireEvent, renderHook } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

import { useReaderHotkeys, type UseReaderHotkeysOptions } from "./useReaderHotkeys"

describe("useReaderHotkeys", () => {
  afterEach(() => document.body.replaceChildren())

  it("opens next and previous entries and marks only unread targets on J/K", () => {
    const options = hotkeyOptions({ cursorEntryId: "first" })
    const firstHook = renderHook(() => useReaderHotkeys(options))

    press("j")
    expect(options.onCursorChange).toHaveBeenCalledWith("second")
    expect(options.onOpenEntry).toHaveBeenCalledWith("second")
    expect(options.onToggleRead).toHaveBeenCalledWith("second")

    firstHook.unmount()
    vi.clearAllMocks()
    const previous = hotkeyOptions({ cursorEntryId: "second", unread: new Set() })
    renderHook(() => useReaderHotkeys(previous))
    press("k")
    expect(previous.onCursorChange).toHaveBeenCalledWith("first")
    expect(previous.onOpenEntry).toHaveBeenCalledWith("first")
    expect(previous.onToggleRead).not.toHaveBeenCalled()
  })

  it("moves the cursor without opening or mutating on N/P", () => {
    const next = hotkeyOptions({ cursorEntryId: "first" })
    const nextHook = renderHook(() => useReaderHotkeys(next))
    press("n")
    expect(next.onCursorChange).toHaveBeenCalledWith("second")
    expect(next.onOpenEntry).not.toHaveBeenCalled()
    expect(next.onToggleRead).not.toHaveBeenCalled()
    expect(next.onToggleStar).not.toHaveBeenCalled()

    nextHook.unmount()
    const previous = hotkeyOptions({ cursorEntryId: "second" })
    renderHook(() => useReaderHotkeys(previous))
    press("p")
    expect(previous.onCursorChange).toHaveBeenCalledWith("first")
    expect(previous.onOpenEntry).not.toHaveBeenCalled()
  })

  it("toggles the cursor entry once on M/S and falls back to the open entry", () => {
    const current = hotkeyOptions({ cursorEntryId: "second" })
    const currentHook = renderHook(() => useReaderHotkeys(current))
    press("m")
    press("s")
    press("m", { repeat: true })
    press("s", { repeat: true })
    expect(current.onToggleRead).toHaveBeenCalledOnce()
    expect(current.onToggleRead).toHaveBeenCalledWith("second")
    expect(current.onToggleStar).toHaveBeenCalledOnce()
    expect(current.onToggleStar).toHaveBeenCalledWith("second")

    currentHook.unmount()
    const fallback = hotkeyOptions({ cursorEntryId: null, openEntryId: "detail-only" })
    renderHook(() => useReaderHotkeys(fallback))
    press("m")
    press("s")
    expect(fallback.onToggleRead).toHaveBeenCalledWith("detail-only")
    expect(fallback.onToggleStar).toHaveBeenCalledWith("detail-only")
  })

  it("opens the current entry with O and marks an unread target", () => {
    const options = hotkeyOptions({ cursorEntryId: "second" })
    renderHook(() => useReaderHotkeys(options))

    press("o")
    press("o", { repeat: true })

    expect(options.onOpenEntry).toHaveBeenCalledOnce()
    expect(options.onOpenEntry).toHaveBeenCalledWith("second")
    expect(options.onToggleRead).toHaveBeenCalledOnce()
    expect(options.onToggleRead).toHaveBeenCalledWith("second")
  })

  it("controls immersive mode, panel focus, and article paging", () => {
    const options = hotkeyOptions({
      canToggleImmersive: true,
      isImmersive: true,
      onToggleImmersive: vi.fn(),
      onExitImmersive: vi.fn(),
      onFocusSources: vi.fn(),
      onFocusQueue: vi.fn(),
      canScrollArticle: true,
      onScrollArticle: vi.fn(),
    })
    renderHook(() => useReaderHotkeys(options))

    press("f")
    press("[")
    press("]")
    press(" ")
    press(" ", { shiftKey: true })
    press("Escape")

    expect(options.onToggleImmersive).toHaveBeenCalledOnce()
    expect(options.onFocusSources).toHaveBeenCalledOnce()
    expect(options.onFocusQueue).toHaveBeenCalledOnce()
    expect(options.onScrollArticle).toHaveBeenNthCalledWith(1, 1)
    expect(options.onScrollArticle).toHaveBeenNthCalledWith(2, -1)
    expect(options.onExitImmersive).toHaveBeenCalledOnce()
  })

  it("keeps Space native on controls", () => {
    const options = hotkeyOptions({ canScrollArticle: true, onScrollArticle: vi.fn() })
    renderHook(() => useReaderHotkeys(options))
    for (const control of [element("button"), element("div", { role: "treeitem", tabindex: "0" })]) {
      document.body.append(control)
      const event = keyEvent(" ")
      expect(control.dispatchEvent(event)).toBe(true)
      expect(event.defaultPrevented).toBe(false)
    }
    expect(options.onScrollArticle).not.toHaveBeenCalled()
  })

  it("does not consume unavailable immersive and article shortcuts", () => {
    const options = hotkeyOptions({
      cursorEntryId: null,
      openEntryId: null,
      canToggleImmersive: false,
      isImmersive: false,
      canScrollArticle: false,
      onToggleImmersive: vi.fn(),
      onExitImmersive: vi.fn(),
      onScrollArticle: vi.fn(),
    })
    renderHook(() => useReaderHotkeys(options))

    for (const key of ["o", "f", "Escape", " "]) {
      const event = keyEvent(key)
      window.dispatchEvent(event)
      expect(event.defaultPrevented).toBe(false)
    }
  })

  it("keeps reading shortcuts active while stale queue actions are disabled", () => {
    const options = hotkeyOptions({
      isQueueDisabled: true,
      canToggleImmersive: true,
      isImmersive: true,
      canScrollArticle: true,
      onToggleImmersive: vi.fn(),
      onExitImmersive: vi.fn(),
      onScrollArticle: vi.fn(),
    })
    renderHook(() => useReaderHotkeys(options))

    const queueEvent = keyEvent("j")
    window.dispatchEvent(queueEvent)
    expect(queueEvent.defaultPrevented).toBe(false)
    expect(options.onCursorChange).not.toHaveBeenCalled()

    press("f")
    press(" ")
    press("Escape")
    expect(options.onToggleImmersive).toHaveBeenCalledOnce()
    expect(options.onScrollArticle).toHaveBeenCalledWith(1)
    expect(options.onExitImmersive).toHaveBeenCalledOnce()
  })

  it("uses the first item without a cursor and never wraps boundaries", () => {
    const first = hotkeyOptions({ cursorEntryId: null, openEntryId: null })
    renderHook(() => useReaderHotkeys(first))
    press("j")
    press("n")
    press("k")
    press("p")
    expect(first.onCursorChange).toHaveBeenNthCalledWith(1, "first")
    expect(first.onCursorChange).toHaveBeenNthCalledWith(2, "first")
    expect(first.onOpenEntry).toHaveBeenCalledOnce()

    const last = hotkeyOptions({ cursorEntryId: "third" })
    renderHook(() => useReaderHotkeys(last))
    press("j")
    press("n")
    expect(last.onCursorChange).not.toHaveBeenCalled()
    expect(last.onOpenEntry).not.toHaveBeenCalled()
  })

  it("allows repeated entry traversal and reserves Shift+J/K for unread sources", () => {
    const options = hotkeyOptions({ cursorEntryId: "first" })
    renderHook(() => useReaderHotkeys(options))
    const repeated = keyEvent("j", { repeat: true })
    window.dispatchEvent(repeated)
    const shiftedNext = keyEvent("J", { shiftKey: true })
    const shiftedPrevious = keyEvent("K", { shiftKey: true })
    const shiftedRepeat = keyEvent("J", { shiftKey: true, repeat: true })
    window.dispatchEvent(shiftedNext)
    window.dispatchEvent(shiftedPrevious)
    window.dispatchEvent(shiftedRepeat)
    expect(options.onCursorChange).toHaveBeenCalledOnce()
    expect(options.onCursorChange).toHaveBeenCalledWith("second")
    expect(options.onNextUnreadSource).toHaveBeenCalledOnce()
    expect(options.onPreviousUnreadSource).toHaveBeenCalledOnce()
    expect(repeated.defaultPrevented).toBe(true)
    expect(shiftedNext.defaultPrevented).toBe(true)
    expect(shiftedPrevious.defaultPrevented).toBe(true)
  })

  it("leaves editable targets and modified shortcuts native", () => {
    const options = hotkeyOptions()
    renderHook(() => useReaderHotkeys(options))
    const targets = [
      element("input"),
      element("textarea"),
      element("select"),
      contentEditable(),
    ]
    for (const target of targets) {
      document.body.append(target)
      const event = keyEvent("j")
      expect(target.dispatchEvent(event)).toBe(true)
      expect(event.defaultPrevented).toBe(false)
    }
    for (const key of ["j", "k", "n", "p", "m", "s"]) {
      for (const modifier of ["ctrlKey", "metaKey", "altKey"] as const) {
        const event = keyEvent(key, { [modifier]: true })
        window.dispatchEvent(event)
        expect(event.defaultPrevented).toBe(false)
      }
    }
    expect(options.onCursorChange).not.toHaveBeenCalled()
  })

  it("lets Escape leave immersive mode from a focused search input", () => {
    const options = hotkeyOptions({
      isImmersive: true,
      onExitImmersive: vi.fn(),
    })
    renderHook(() => useReaderHotkeys(options))
    const input = element("input", { type: "search" })
    document.body.append(input)
    input.focus()

    input.dispatchEvent(keyEvent("Escape"))

    expect(options.onExitImmersive).toHaveBeenCalledOnce()
  })

  it("keeps Escape native while an input method editor is composing", () => {
    const options = hotkeyOptions({
      isImmersive: true,
      onExitImmersive: vi.fn(),
    })
    renderHook(() => useReaderHotkeys(options))
    const input = element("input", { type: "search" })
    document.body.append(input)
    input.focus()
    const composingEscape = keyEvent("Escape", { isComposing: true })

    expect(input.dispatchEvent(composingEscape)).toBe(true)
    expect(composingEscape.defaultPrevented).toBe(false)
    expect(options.onExitImmersive).not.toHaveBeenCalled()
  })

  it.each(["textbox", "searchbox", "combobox", "spinbutton", "slider"])(
    "leaves a focused %s role and its descendants native",
    (role) => {
      const options = hotkeyOptions()
      renderHook(() => useReaderHotkeys(options))
      const editable = element("div", { role, tabindex: "0" })
      const child = element("span", { tabindex: "0" })
      editable.append(child)
      document.body.append(editable)
      fireEvent.focusIn(child)

      const event = keyEvent("j")
      expect(child.dispatchEvent(event)).toBe(true)
      expect(event.defaultPrevented).toBe(false)
      expect(options.onCursorChange).not.toHaveBeenCalled()
    },
  )

  it("leaves shortcuts native while a controlled modal dialog is open", () => {
    const options = hotkeyOptions()
    const hook = renderHook((props) => useReaderHotkeys(props), { initialProps: options })
    const dialog = element("div", { role: "dialog", "aria-modal": "true" })
    const button = element("button")
    dialog.append(button)
    document.body.append(dialog)
    hook.rerender({ ...options, isDisabled: true })

    const event = keyEvent("m")
    expect(button.dispatchEvent(event)).toBe(true)
    expect(event.defaultPrevented).toBe(false)
    expect(options.onToggleRead).not.toHaveBeenCalled()
  })

  it.each(["native", "aria"])(
    "blocks an immediately opened %s modal before ASTRYX prevents default",
    (kind) => {
      const options = hotkeyOptions()
      renderHook(() => useReaderHotkeys(options))
      const dialog = kind === "native"
        ? element("dialog", { open: "" })
        : element("div", { role: "dialog", "aria-modal": "true" })
      const button = element("button")
      dialog.append(button)
      document.body.append(dialog)

      const event = keyEvent("j")
      expect(button.dispatchEvent(event)).toBe(true)
      expect(event.defaultPrevented).toBe(false)
      expect(options.onCursorChange).not.toHaveBeenCalled()
    },
  )

  it("does not block shortcuts for a closed native alert dialog", () => {
    const options = hotkeyOptions()
    renderHook(() => useReaderHotkeys(options))
    document.body.append(element("dialog", {
      role: "alertdialog",
      "aria-modal": "true",
    }))

    press("j")
    expect(options.onCursorChange).toHaveBeenCalledWith("second")
  })

  it("does not block shortcuts for closed popover dialog content", () => {
    const options = hotkeyOptions()
    renderHook(() => useReaderHotkeys(options))
    const popover = element("div", { popover: "auto" })
    popover.append(element("div", { role: "dialog", "aria-modal": "true" }))
    document.body.append(popover)

    press("j")
    expect(options.onCursorChange).toHaveBeenCalledWith("second")
  })
})

function hotkeyOptions(overrides: Partial<UseReaderHotkeysOptions> & { unread?: Set<string> } = {}): UseReaderHotkeysOptions {
  const unread = overrides.unread ?? new Set(["first", "second", "third"])
  return {
    queueEntryIds: ["first", "second", "third"],
    cursorEntryId: "first",
    openEntryId: "first",
    isDisabled: false,
    isUnread: (entryId) => unread.has(entryId),
    onCursorChange: vi.fn(),
    onOpenEntry: vi.fn(),
    onToggleRead: vi.fn(),
    onToggleStar: vi.fn(),
    onNextUnreadSource: vi.fn(),
    onPreviousUnreadSource: vi.fn(),
    ...overrides,
  }
}

function press(key: string, init: KeyboardEventInit = {}) {
  fireEvent.keyDown(window, { key, ...init })
}

function keyEvent(key: string, init: KeyboardEventInit = {}) {
  return new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...init })
}

function element(tag: string, attributes: Record<string, string> = {}) {
  const node = document.createElement(tag)
  for (const [name, value] of Object.entries(attributes)) node.setAttribute(name, value)
  return node
}

function contentEditable() {
  const node = element("div", { contenteditable: "true" })
  Object.defineProperty(node, "isContentEditable", { configurable: true, value: true })
  return node
}
