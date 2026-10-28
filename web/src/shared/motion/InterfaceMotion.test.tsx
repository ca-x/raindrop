import { fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it } from "vitest"

import { InterfaceMotion } from "./InterfaceMotion"

describe("interface motion input modality", () => {
  afterEach(() => {
    delete document.documentElement.dataset.raindropInput
  })

  it("keeps content available without an animation gate", () => {
    render(<InterfaceMotion><button>Read article</button></InterfaceMotion>)

    expect(screen.getByRole("button", { name: "Read article" })).toBeEnabled()
  })

  it("distinguishes pointer input from keyboard navigation", () => {
    render(<InterfaceMotion><button>Read article</button></InterfaceMotion>)
    const button = screen.getByRole("button")

    fireEvent.pointerDown(button)
    expect(document.documentElement).toHaveAttribute("data-raindrop-input", "pointer")

    fireEvent.keyDown(button, { key: "Enter" })
    expect(document.documentElement).toHaveAttribute("data-raindrop-input", "keyboard")

    fireEvent.pointerDown(button)
    expect(document.documentElement).toHaveAttribute("data-raindrop-input", "pointer")
  })

  it("treats virtual clicks as non-motion input", () => {
    render(<InterfaceMotion><button>Open settings</button></InterfaceMotion>)

    fireEvent.click(screen.getByRole("button"), { detail: 0 })

    expect(document.documentElement).toHaveAttribute("data-raindrop-input", "keyboard")
  })

  it("does not rewrite the root attribute for repeated keyboard input", () => {
    render(<InterfaceMotion><button>Read article</button></InterfaceMotion>)
    const observer = new MutationObserver(() => {})
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ["data-raindrop-input"] })

    fireEvent.keyDown(screen.getByRole("button"), { key: "j" })
    fireEvent.keyDown(screen.getByRole("button"), { key: "j", repeat: true })

    expect(observer.takeRecords()).toHaveLength(1)
    observer.disconnect()
  })

  it("does not track events after unmounting", () => {
    const { unmount } = render(<InterfaceMotion>Reader</InterfaceMotion>)
    unmount()

    fireEvent.pointerDown(document.body)

    expect(document.documentElement).not.toHaveAttribute("data-raindrop-input")
  })
})
