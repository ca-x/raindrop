import { render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { expect, it, vi } from "vitest"

import { Providers } from "../../../app/Providers"
import { ApiClientError } from "../../../shared/api/client"
import { activateLocale } from "../../../shared/i18n/i18n"
import type { OpmlTransferApi } from "./OpmlTransferPanel"
import { OpmlTransferPanel } from "./OpmlTransferPanel"

const preview = {
  mode: "PREVIEW",
  outlineCount: 5,
  validCount: 2,
  newCount: 2,
  importedCount: 0,
  duplicateCount: 1,
  invalidCount: 1,
  categoryCount: 1,
  createdCategoryCount: 0,
} as const

it("previews a selected file before enabling an explicit import", async () => {
  activateLocale("en")
  const user = userEvent.setup()
  const onImported = vi.fn().mockResolvedValue(undefined)
  const api = fakeApi({
    previewOpml: vi.fn().mockResolvedValue(preview),
    commitOpml: vi.fn().mockResolvedValue({
      ...preview,
      mode: "COMMIT",
      importedCount: 2,
    }),
  })
  renderPanel(api, onImported)
  const file = new File(["<opml/>"] , "subscriptions.opml", {
    type: "application/xml",
  })

  await user.upload(fileInput(), file)

  expect(await screen.findByText("New")).toBeVisible()
  const newStat = screen.getByText("New").closest<HTMLElement>(".reader-opml-stat")!
  expect(within(newStat).getByText("2")).toBeVisible()
  const importButton = screen.getByRole("button", { name: "Import subscriptions" })
  expect(importButton).toBeEnabled()
  await user.click(importButton)

  expect(api.commitOpml).toHaveBeenCalledWith(file, "csrf-memory")
  expect(onImported).toHaveBeenCalledOnce()
  expect(await screen.findByText("Imported 2 subscriptions")).toBeVisible()
})

it("shows the bounded server validation message when preview fails", async () => {
  activateLocale("en")
  const user = userEvent.setup()
  const api = fakeApi({
    previewOpml: vi.fn().mockRejectedValue(
      new ApiClientError(422, {
        code: "VALIDATION_ERROR",
        message: "Request validation failed",
        fields: { file: "OPML file is malformed" },
      }),
    ),
  })
  renderPanel(api)

  await user.upload(
    fileInput(),
    new File(["<broken>"], "broken.opml", { type: "application/xml" }),
  )

  expect(await screen.findByText("OPML file is malformed")).toBeVisible()
  expect(screen.getByRole("button", { name: "Import subscriptions" })).toBeDisabled()
})

function renderPanel(api: OpmlTransferApi, onImported = vi.fn()) {
  return render(
    <Providers>
      <OpmlTransferPanel
        csrfToken="csrf-memory"
        onImported={onImported}
        api={api}
      />
    </Providers>,
  )
}

function fileInput(): HTMLInputElement {
  const input = screen
    .getAllByLabelText("OPML file")
    .find((element): element is HTMLInputElement => element instanceof HTMLInputElement)
  if (!input) throw new Error("OPML file input was not rendered")
  return input
}

function fakeApi(overrides: Partial<OpmlTransferApi>): OpmlTransferApi {
  return {
    previewOpml: vi.fn(),
    commitOpml: vi.fn(),
    exportOpml: vi.fn(),
    ...overrides,
  }
}
