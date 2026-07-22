import { fireEvent, render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { expect, it, vi } from "vitest"

import { Providers } from "../../../app/Providers"
import { activateLocale } from "../../../shared/i18n/i18n"
import type { EntryTranslationController } from "../model/useEntryTranslationController"
import { ArticleSelectionPopover } from "./ArticleSelectionPopover"
import {
  readSelectedArticleAnchor,
  readSelectedArticleText,
} from "./articleSelection"

it("reads only selections whose endpoints stay inside the article body", () => {
  const body = document.createElement("div")
  const paragraph = document.createElement("p")
  paragraph.textContent = "Selected paragraph"
  body.append(paragraph)
  const outside = document.createElement("p")
  outside.textContent = "Outside"
  document.body.append(body, outside)

  const selection = document.getSelection()
  const insideRange = document.createRange()
  insideRange.selectNodeContents(paragraph)
  selection?.removeAllRanges()
  selection?.addRange(insideRange)
  expect(readSelectedArticleText(body, selection)).toBe("Selected paragraph")

  const crossingRange = document.createRange()
  crossingRange.setStart(paragraph.firstChild!, 0)
  crossingRange.setEnd(outside.firstChild!, 7)
  selection?.removeAllRanges()
  selection?.addRange(crossingRange)
  expect(readSelectedArticleText(body, selection)).toBe("")

  const outsideRange = document.createRange()
  outsideRange.selectNodeContents(outside)
  const multiRangeSelection = {
    isCollapsed: false,
    rangeCount: 2,
    getRangeAt: (index: number) => [insideRange, outsideRange][index]!,
    toString: () => "Selected paragraphOutside",
  } as unknown as Selection
  expect(readSelectedArticleText(body, multiRangeSelection)).toBe("")

  paragraph.textContent = "unsafe\u0007text"
  const controlRange = document.createRange()
  controlRange.selectNodeContents(paragraph)
  selection?.removeAllRanges()
  selection?.addRange(controlRange)
  expect(readSelectedArticleText(body, selection)).toBe("")

  selection?.removeAllRanges()
  body.remove()
  outside.remove()
})

it("anchors the floating action to the final visible selection rectangle", () => {
  const body = document.createElement("div")
  const paragraph = document.createElement("p")
  paragraph.textContent = "A selection that wraps onto another line"
  body.append(paragraph)
  document.body.append(body)
  const range = document.createRange()
  range.selectNodeContents(paragraph)
  Object.defineProperty(range, "getClientRects", {
    value: () =>
      [
        new DOMRect(10, 20, 80, 18),
        new DOMRect(10, 40, 52, 18),
      ] as unknown as DOMRectList,
  })
  const selection = document.getSelection()
  selection?.removeAllRanges()
  selection?.addRange(range)

  expect(readSelectedArticleAnchor(body, selection)).toEqual({
    clientX: 62,
    clientY: 58,
  })

  selection?.removeAllRanges()
  body.remove()
})

it("offers a floating action after selection and looks up only after activation", async () => {
  activateLocale("en")
  const user = userEvent.setup()
  const controller = createController()
  renderPopover("quick brown fox", controller)

  revealSelectionAction()

  const selectionAction = await screen.findByRole("button", {
    name: "Look up or translate selected text",
  })
  const dialog = screen.getByRole("dialog", {
    name: "Selected text translation",
    hidden: true,
  })
  expect(controller.lookup).not.toHaveBeenCalled()
  expect(dialog.closest("[popover]")).not.toHaveAttribute("popover-open")

  await user.click(selectionAction)

  expect(controller.lookup).toHaveBeenCalledWith("quick brown fox")
  expect(
    await screen.findByRole("dialog", {
      name: "Selected text translation",
      hidden: true,
    }),
  ).toBeInTheDocument()
  expect(
    screen.getByRole("group", {
      name: "Selected text mode",
      hidden: true,
    }),
  ).toBeInTheDocument()
  expect(screen.queryByRole("textbox", { hidden: true })).not.toBeInTheDocument()
})

it("opens paragraph translation directly and omits word lookup mode", async () => {
  activateLocale("en")
  const user = userEvent.setup()
  const selectedText = "x".repeat(201)
  const controller = createController()
  renderPopover(selectedText, controller)

  revealSelectionAction()
  await user.click(
    await screen.findByRole("button", {
      name: "Look up or translate selected text",
    }),
  )

  expect(controller.translateSelection).toHaveBeenCalledWith(selectedText)
  expect(
    await screen.findByRole("dialog", {
      name: "Selected text translation",
      hidden: true,
    }),
  ).toBeInTheDocument()
  expect(
    screen.queryByRole("group", {
      name: "Selected text mode",
      hidden: true,
    }),
  ).not.toBeInTheDocument()
})

it("lets a short selection switch from lookup to translation in place", async () => {
  activateLocale("en")
  const user = userEvent.setup()
  const controller = createController()
  renderPopover("fox", controller)
  await openSelectedTextPopover()

  await user.click(
    await screen.findByRole("button", {
      name: "Translate selection",
      hidden: true,
    }),
  )
  expect(controller.translateSelection).toHaveBeenCalledWith("fox")
})

it("preserves the native context menu for a valid selection", () => {
  activateLocale("en")
  const controller = createController()
  renderPopover("selected text", controller)
  const articleBody = selectArticleText()

  expect(fireEvent.contextMenu(articleBody)).toBe(true)
  expect(controller.lookup).not.toHaveBeenCalled()
  expect(controller.translateSelection).not.toHaveBeenCalled()
})

it("does not offer an action when the selection is too large", () => {
  activateLocale("en")
  const controller = createController()
  renderPopover("x".repeat(8_001), controller)

  revealSelectionAction()
  expect(
    screen.queryByRole("button", {
      name: "Look up or translate selected text",
    }),
  ).not.toBeInTheDocument()
  expect(controller.lookup).not.toHaveBeenCalled()
})

it("preserves the native context menu when body text is not selected", () => {
  activateLocale("en")
  const controller = createController()
  renderPopover("Article body", controller)
  document.getSelection()?.removeAllRanges()

  expect(
    fireEvent.contextMenu(screen.getByTestId("article-body"), {
      clientX: 40,
      clientY: 40,
    }),
  ).toBe(true)
  expect(controller.lookup).not.toHaveBeenCalled()
  expect(controller.translateSelection).not.toHaveBeenCalled()
})

it("offers the same action after a mobile-style stable selection", async () => {
  activateLocale("en")
  const controller = createController()
  renderPopover("mobile selection", controller)
  selectArticleText()

  fireEvent(document, new Event("selectionchange"))

  expect(
    await screen.findByRole("button", {
      name: "Look up or translate selected text",
    }),
  ).toBeInTheDocument()
  expect(controller.lookup).not.toHaveBeenCalled()
})

it("dismisses and cancels the floating result with Escape", async () => {
  activateLocale("en")
  const controller = createController()
  renderPopover("fox", controller)
  await openSelectedTextPopover()
  const dialog = await screen.findByRole("dialog", { hidden: true })
  expect(dialog.closest("[popover]")).toHaveAttribute("popover-open")

  fireEvent.keyDown(document, { key: "Escape" })
  await waitFor(() =>
    expect(dialog.closest("[popover]")).not.toHaveAttribute("popover-open"),
  )
  expect(controller.cancelContextActions).toHaveBeenCalled()
})

it("keeps selected-text failures inside the floating result", async () => {
  activateLocale("en")
  const controller = createController({
    contextError: "RATE_LIMITED",
  })
  renderPopover("fox", controller)
  await openSelectedTextPopover()

  expect(await screen.findByRole("alert", { hidden: true })).toHaveTextContent(
    "Too many requests. Try again later.",
  )
})

it("closes and disables the prior selection while the reader changes entry", async () => {
  activateLocale("en")
  const controller = createController()
  const view = renderPopover("fox", controller)
  await openSelectedTextPopover()
  const dialog = await screen.findByRole("dialog", { hidden: true })
  expect(dialog.closest("[popover]")).toHaveAttribute("popover-open")

  view.rerender(
    <Providers>
      <ArticleSelectionPopover
        controller={{ ...controller, entryId: null }}
        isEnabled
      >
        <p data-testid="article-body">fox</p>
      </ArticleSelectionPopover>
    </Providers>,
  )

  expect(dialog.closest("[popover]")).not.toHaveAttribute("popover-open")
  expect(controller.cancelContextActions).toHaveBeenCalled()
  revealSelectionAction()
  expect(controller.lookup).toHaveBeenCalledTimes(1)
})

it("renders provider output as text instead of executable markup", async () => {
  activateLocale("en")
  const translatedText = '<img src="x" onerror="window.__unsafe=true">'
  const controller = createController({
    selectionResult: {
      translatedText,
      providerLabel: "DeepLX",
      detectedSourceLocale: "en",
      targetLocale: "zh-CN",
    },
  })
  renderPopover("x".repeat(201), controller)
  await openSelectedTextPopover()

  expect(await screen.findByText(translatedText, { exact: true })).toBeInTheDocument()
  expect(
    document.querySelector(".reader-selection-translation-text img"),
  ).toBeNull()
})

function renderPopover(
  selectedText: string,
  controller: EntryTranslationController,
) {
  return render(
    <Providers>
      <ArticleSelectionPopover
        controller={controller}
        isEnabled
      >
        <p data-testid="article-body">{selectedText}</p>
      </ArticleSelectionPopover>
    </Providers>,
  )
}

function selectArticleText(): HTMLElement {
  const articleBody = screen.getByTestId("article-body")
  const range = document.createRange()
  range.selectNodeContents(articleBody)
  const selection = document.getSelection()
  selection?.removeAllRanges()
  selection?.addRange(range)
  return articleBody
}

function revealSelectionAction(): boolean {
  return fireEvent.pointerUp(selectArticleText(), {
    clientX: 40,
    clientY: 40,
  })
}

async function openSelectedTextPopover(): Promise<void> {
  revealSelectionAction()
  const selectionAction = await screen.findByRole("button", {
    name: "Look up or translate selected text",
  })
  fireEvent.pointerDown(selectionAction)
  fireEvent.click(selectionAction)
}

function createController(
  overrides: Partial<EntryTranslationController> = {},
): EntryTranslationController {
  return {
    entryId: "00000000-0000-4000-8000-000000000301",
    result: null,
    lookupResult: null,
    selectionResult: null,
    isTranslating: false,
    isLookingUp: false,
    isTranslatingSelection: false,
    completedSegments: 0,
    totalSegments: 0,
    articleError: null,
    contextError: null,
    translate: vi.fn().mockResolvedValue(true),
    lookup: vi.fn().mockResolvedValue(true),
    translateSelection: vi.fn().mockResolvedValue(true),
    clearTranslation: vi.fn(),
    clearLookup: vi.fn(),
    clearSelectionTranslation: vi.fn(),
    cancelContextActions: vi.fn(),
    clearError: vi.fn(),
    ...overrides,
  }
}
