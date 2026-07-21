import { expect, type Locator, type Page } from "@playwright/test"

import { readerIds, type ReaderApiFixture } from "./readerApiFixture"
import {
  expectDialogContained,
  expectNoHorizontalOverflow,
  readerRow,
  readerRowButton,
} from "./readerAssertions"

export async function verifyWideCategoryWorkflow(
  page: Page,
  fixture: ReaderApiFixture,
  baseURL: string,
): Promise<void> {
  await page
    .getByRole("navigation", { name: "Sources" })
    .getByRole("button", { name: "Quiet Web", exact: true })
    .click()
  await expect(page).toHaveURL(`${baseURL}/reader/feed/${readerIds.feedA}`)
  await page.getByRole("button", { name: "Manage subscriptions" }).click()
  const dialog = page.getByRole("dialog", { name: "Manage subscriptions" })
  await expect(dialog).toBeVisible()
  await dialog.getByRole("button", { name: "Add category" }).click()

  await dialog.getByRole("textbox", { name: /^New category/u }).fill("Reading")
  await dialog.getByRole("button", { name: "Create category" }).click()
  await dialog.getByRole("button", { name: /Reading/u }).click()
  await dialog.getByRole("textbox", { name: /^Category name/u }).fill("Research")
  await dialog.getByRole("button", { name: "Save changes" }).click()
  await expect(dialog.getByRole("button", { name: /Research/u })).toBeVisible()

  await dialog.getByRole("button", { name: "Close" }).click()
  await page.getByRole("button", { name: "Edit current subscription" }).click()
  const editDialog = page.getByRole("dialog", { name: "Edit current subscription" })
  const selector = editDialog.getByRole("combobox", {
    name: /^Category for the current feed/u,
  })
  await selector.click()
  await page.getByRole("option", { name: "Research" }).click()
  await expect(selector).toContainText("Research")
  await editDialog.getByRole("button", { name: "Close" }).click()

  const sources = page.getByRole("navigation", { name: "Sources" })
  await sources.getByRole("button", { name: "Research", exact: true }).click()
  await expect(page).toHaveURL(`${baseURL}/reader/category/${readerIds.categoryB}`)
  await expect(readerRow(page, readerIds.firstEntry)).toBeVisible()
  await expect(readerRow(page, readerIds.seventhEntry)).toHaveCount(0)

  await page.getByRole("button", { name: "Manage subscriptions" }).click()
  await dialog.getByRole("button", { name: "Add category" }).click()
  await dialog.getByRole("button", { name: /Research/u }).click()
  await dialog.getByRole("button", { name: "Delete category" }).click()
  const alert = page.getByRole("alertdialog", { name: "Delete this category?" })
  await alert.getByRole("button", { name: "Delete category" }).click()
  await expect(page).toHaveURL(`${baseURL}/reader/unread`)
  await expect(
    sources.getByRole("button", { name: "Research", exact: true }),
  ).toHaveCount(0)

  expect(fixture.organization.categoryCalls.map((call) => call.method)).toEqual([
    "POST",
    "PATCH",
    "DELETE",
  ])
  expect(fixture.organization.categoryCalls.every((call) => Boolean(call.csrf))).toBe(true)
  expect(fixture.organization.subscriptionPatches).toHaveLength(1)
  expect(fixture.organization.subscriptionPatches[0]).toMatchObject({
    subscriptionId: readerIds.subscriptionA,
    body: { categoryId: readerIds.categoryB },
  })
  expect(fixture.organization.subscriptions[0].categoryId).toBeNull()
  await verifyCrossUserDenial(page, fixture)
}

export async function verifyMediumCategoryFocus(page: Page): Promise<void> {
  const sources = page.getByRole("dialog", { name: "Sources" })
  await page.getByRole("button", { name: "Open sources" }).click()
  await sources.getByRole("button", { name: "Manage subscriptions" }).click()
  const managementDialog = page.getByRole("dialog", { name: "Manage subscriptions" })
  await expect(managementDialog).toBeVisible()
  await expect(sources).not.toBeVisible()
  await managementDialog.getByRole("button", { name: "Close" }).click()
  await expect(sources).toBeVisible()
  await expect(sources.getByRole("button", { name: "Manage subscriptions" })).toBeFocused()
}

export async function verifyCompactCategoryRoute(
  page: Page,
  baseURL: string,
): Promise<void> {
  await page.goto(`${baseURL}/reader/category/${readerIds.categoryA}`)
  await expect(readerRow(page, readerIds.seventhEntry)).toBeVisible()
  await expect(readerRow(page, readerIds.firstEntry)).toHaveCount(0)
  await readerRowButton(page, readerIds.seventhEntry).click()
  await expect(page).toHaveURL(
    `${baseURL}/reader/category/${readerIds.categoryA}/entry/${readerIds.seventhEntry}`,
  )
  await page.goBack()
  await expect(page).toHaveURL(`${baseURL}/reader/category/${readerIds.categoryA}`)
  await page.goForward()
  await page.getByRole("button", { name: "Back to entry queue" }).click()
  await expect(page).toHaveURL(`${baseURL}/reader/category/${readerIds.categoryA}`)
  await verifyCompactCategoryDialog(page)
}

export async function verifyDirectCompactCategory(
  page: Page,
  baseURL: string,
): Promise<void> {
  await page.goto(
    `${baseURL}/reader/category/${readerIds.categoryA}/entry/${readerIds.seventhEntry}`,
  )
  await expect(page.getByRole("heading", { name: "Fixture entry 07" })).toBeVisible()
  await page.getByRole("button", { name: "Back to entry queue" }).click()
  await expect(page).toHaveURL(`${baseURL}/reader/category/${readerIds.categoryA}`)
  await verifyCompactCategoryDialog(page)
}

async function verifyCrossUserDenial(
  page: Page,
  fixture: ReaderApiFixture,
): Promise<void> {
  const csrf = fixture.organization.categoryCalls[0]?.csrf
  if (!csrf) throw new Error("expected a captured organization CSRF token")
  const statuses = await page.evaluate(
    async ({ categoryId, subscriptionId, csrfToken }) =>
      Promise.all([
        fetch(`/api/v1/categories/${categoryId}`, {
          method: "PATCH",
          headers: {
            "content-type": "application/json",
            "x-csrf-token": csrfToken,
          },
          body: JSON.stringify({ title: "Hidden" }),
        }).then((response) => response.status),
        fetch(`/api/v1/subscriptions/${subscriptionId}`, {
          method: "PATCH",
          headers: {
            "content-type": "application/json",
            "x-csrf-token": csrfToken,
          },
          body: JSON.stringify({ categoryId: null }),
        }).then((response) => response.status),
      ]),
    {
      categoryId: readerIds.otherUserCategory,
      subscriptionId: readerIds.otherUserSubscription,
      csrfToken: csrf,
    },
  )
  expect(statuses).toEqual([404, 404])
}

async function verifyCompactCategoryDialog(page: Page): Promise<void> {
  await page.getByRole("button", { name: "Open sources" }).click()
  const sources = page.getByRole("dialog", { name: "Sources" })
  await expectCategoryRowAligned(
    sources.locator(`[data-tree-id="category:${readerIds.categoryA}"]`),
  )
  await sources.getByRole("button", { name: "Manage subscriptions" }).click()
  const dialog = page.getByRole("dialog", { name: "Manage subscriptions" })
  await expectDialogContained(dialog, page)
  await expectNoHorizontalOverflow(page)
  await dialog.getByRole("button", { name: "Close" }).click()
  await expect(sources).toBeVisible()
  await expect(sources.getByRole("button", { name: "Manage subscriptions" })).toBeFocused()
}

async function expectCategoryRowAligned(row: Locator): Promise<void> {
  const boxes = await Promise.all([
    row.getByRole("button", { name: "Toggle children" }).boundingBox(),
    row.locator(".reader-smart-source-icon").first().boundingBox(),
    row.locator(".reader-source-label").boundingBox(),
  ])
  expect(boxes.every(Boolean)).toBe(true)
  const centers = boxes.map((box) => box!.y + box!.height / 2)
  expect(Math.max(...centers) - Math.min(...centers)).toBeLessThanOrEqual(1)
}
