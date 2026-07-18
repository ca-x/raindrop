import { expect, type Page } from "@playwright/test"

import { readerIds, type ReaderApiFixture } from "./readerApiFixture"
import {
  expectNoHorizontalOverflow,
  readerRow,
  readerRowButton,
} from "./readerAssertions"

export async function verifyWideUnreadSourceHotkeys(
  page: Page,
  baseURL: string,
): Promise<void> {
  await expect(page.getByText("Rust Dispatch", { exact: true })).toBeVisible()
  for (const key of ["Shift + J", "Shift + K"]) {
    await expect(page.getByRole("img", { name: key, exact: true })).toBeVisible()
  }

  await page.keyboard.press("Shift+J")
  await expect(page).toHaveURL(`${baseURL}/reader/feed/${readerIds.feedB}`)
  await expect(readerRow(page, readerIds.firstEntry)).toHaveCount(0)
  await expect(readerRowButton(page, readerIds.seventhEntry)).toBeEnabled()
  await page.keyboard.press("Shift+K")
  await expect(page).toHaveURL(`${baseURL}/reader/unread`)
  await expect(readerRowButton(page, readerIds.firstEntry)).toBeEnabled()
}

export async function verifyMediumFeedSearchAndMenu(
  page: Page,
  fixture: ReaderApiFixture,
  baseURL: string,
): Promise<void> {
  const sourcesTrigger = page.getByRole("button", { name: "Open sources" })
  await sourcesTrigger.click()
  await page.getByRole("dialog", { name: "Sources" })
    .getByText("Quiet Web", { exact: true })
    .click()
  await expect(page).toHaveURL(`${baseURL}/reader/feed/${readerIds.feedA}`)

  const search = page.getByRole("textbox", { name: "Search this feed" })
  await search.fill("Second quiet")
  await search.press("Enter")
  await expect.poll(() => fixture.entryLists.at(-1)?.search).toBe("Second quiet")
  await expect(readerRow(page, readerIds.secondEntry)).toBeVisible()
  await expect(readerRow(page, readerIds.firstEntry)).toHaveCount(0)

  await page.getByRole("button", { name: "Queue menu" }).click()
  await expect(page.getByRole("menuitem", {
    name: "Mark all is unavailable in this view",
  })).toHaveAttribute("aria-disabled", "true")
  await page.keyboard.press("Escape")

  await page.getByRole("button", { name: "Clear Search this feed" }).click()
  await expect.poll(() => fixture.entryLists.at(-1)?.search).toBeNull()
  await expect(readerRow(page, readerIds.firstEntry)).toBeVisible()

  await page.getByRole("button", { name: "Queue menu" }).click()
  await page.getByRole("menuitem", { name: "Previous unread source" }).click()
  await expect(page).toHaveURL(`${baseURL}/reader/feed/${readerIds.feedB}`)
  await expectNoHorizontalOverflow(page)
}

export async function verifyCompactQueueMenu(page: Page): Promise<void> {
  const trigger = page.getByRole("button", { name: "Queue menu" })
  await trigger.click()
  for (const action of [
    "Next unread source",
    "Previous unread source",
    "Mark current source read",
  ]) {
    await expect(page.getByRole("menuitem", { name: action })).toBeVisible()
  }
  await page.keyboard.press("Escape")
  await expect(trigger).toBeFocused()
  await expectNoHorizontalOverflow(page)
}

export async function verifyStableSnapshotBulkRead(
  page: Page,
  fixture: ReaderApiFixture,
  baseURL: string,
): Promise<void> {
  await page.goto(`${baseURL}/reader/unread`)
  await expect(readerRow(page, readerIds.pendingEntry)).toBeVisible()

  fixture.discoverLatePending()
  await page.getByRole("button", { name: "Reload stored entries" }).click()
  const pendingNotice = page.getByRole("status").filter({
    hasText: "1 new entries available",
  })
  await expect(pendingNotice).toBeVisible()

  await page.getByRole("button", { name: "Queue menu" }).click()
  await page.getByRole("menuitem", { name: "Mark current source read" }).click()
  const dialog = page.getByRole("alertdialog", { name: "Mark “Unread” read?" })
  await expect(dialog).toContainText(
    "Entries received after the list loaded will stay unread.",
  )
  await expect(dialog.getByRole("button", { name: "Cancel" })).toBeFocused()
  await dialog.getByRole("button", { name: "Mark all read" }).click()

  await expect.poll(() => fixture.markReadCalls.length).toBe(1)
  expect(fixture.markReadCalls[0]).toMatchObject({
    body: { snapshotGeneration: 2 },
  })
  expect(fixture.markReadCalls[0]?.csrf).toBeTruthy()
  await expect.poll(() => fixture.entryState(readerIds.pendingEntry).isRead).toBe(true)
  await expect.poll(() => fixture.entryState(readerIds.latePendingEntry).isRead).toBe(false)
  await expect(dialog).not.toBeVisible()
  await expect(readerRow(page, readerIds.pendingEntry)).toHaveCount(0)
  await expect(readerRow(page, readerIds.latePendingEntry)).toBeVisible()
  await expectNoHorizontalOverflow(page)
}
