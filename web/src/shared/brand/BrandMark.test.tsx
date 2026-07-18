import { render, screen } from "@testing-library/react"
import { expect, it } from "vitest"

import { BrandMark } from "./BrandMark"
import { ReadyPage } from "../../features/reader/ReadyPage"
import { Providers } from "../../app/Providers"
import { setTestViewport } from "../../test/setup"

it("renders an undistorted semantic brand image and supports decorative reuse", () => {
  const { container } = render(
    <>
      <BrandMark size="sm" />
      <BrandMark size="lg" decorative />
    </>,
  )

  const logo = screen.getByRole("img", { name: "Raindrop" })
  expect(logo).toHaveAttribute("width", "32")
  expect(logo).toHaveAttribute("height", "32")
  expect(logo).toHaveAttribute("src", "/brand/raindrop-logo-192.png")
  expect(logo).toHaveAttribute("srcset", expect.stringContaining("raindrop-logo-512.png"))

  const decorative = container.querySelector('img[aria-hidden="true"]')
  expect(decorative).toHaveAttribute("alt", "")
  expect(decorative).toHaveAttribute("width", "72")
  expect(decorative).toHaveAttribute("height", "72")
})

it("uses one semantic brand image in the compact desktop shell toolbar", () => {
  setTestViewport(1280, 800)
  render(
    <Providers>
      <ReadyPage
        session={{
          user: { id: "1", username: "Reader", email: null, isDisabled: false, roles: ["USER"] },
          csrfToken: "csrf",
          expiresAt: "2026-08-15T08:00:00Z",
        }}
        onLoggedOut={() => undefined}
      />
    </Providers>,
  )

  expect(screen.getByRole("img", { name: "Raindrop" })).toBeVisible()
  expect(screen.queryByText("Raindrop")).not.toBeInTheDocument()
})
