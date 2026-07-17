import { expect, type Locator, type Page } from "@playwright/test"

export function readerRow(page: Page, entryId: string): Locator {
  return page.locator(`[data-reader-entry-id="${entryId}"]`)
}

export function readerRowButton(page: Page, entryId: string): Locator {
  return readerRow(page, entryId).locator("button")
}

export async function expectNoHorizontalOverflow(page: Page): Promise<void> {
  await expect.poll(() => page.evaluate(() => ({
    documentFits: document.documentElement.scrollWidth <= window.innerWidth,
    bodyFits: document.body.scrollWidth <= window.innerWidth,
  }))).toEqual({ documentFits: true, bodyFits: true })
}

export async function setScrollTop(locator: Locator, offset: number): Promise<number> {
  return locator.evaluate((element, value) => {
    const maximum = element.scrollHeight - element.clientHeight
    if (maximum <= 0) throw new Error("expected element to have a vertical scroll range")
    element.scrollTop = Math.min(value, maximum)
    element.dispatchEvent(new Event("scroll", { bubbles: true }))
    return element.scrollTop
  }, offset)
}

export async function expectScrollTop(locator: Locator, expected: number, tolerance = 4): Promise<void> {
  await expect.poll(() => locator.evaluate((element) => element.scrollTop)).toBeGreaterThanOrEqual(expected - tolerance)
  await expect.poll(() => locator.evaluate((element) => element.scrollTop)).toBeLessThanOrEqual(expected + tolerance)
}

export async function expectHostileContentContained(page: Page): Promise<void> {
  const body = page.locator(".reader-article-body")
  const inert = body.locator('img[data-raindrop-inert-image="0"]')
  await expect(inert).not.toHaveAttribute("src", /.+/u)
  await expect(body).not.toContainText("publisher.invalid/tracker.gif")
  await expect.poll(() => body.evaluate((element) => !element.innerHTML.includes("publisher.invalid/tracker.gif"))).toBe(true)

  for (const selector of [
    '[data-fixture="wide-table"]',
    '[data-fixture="wide-pre"]',
    '[data-fixture="wide-iframe"]',
    '[data-fixture="wide-video"]',
    'img[data-raindrop-inert-image="0"]',
  ]) {
    await expect.poll(() => body.locator(selector).evaluate((element) => {
      const box = element.getBoundingClientRect()
      const container = element.closest(".reader-article-body")?.getBoundingClientRect()
      return Boolean(container && box.left >= container.left - 1 && box.right <= container.right + 1)
    })).toBe(true)
  }

  for (const selector of ['[data-fixture="wide-table"]', '[data-fixture="wide-pre"]']) {
    await expect.poll(() => body.locator(selector).evaluate((element) => getComputedStyle(element).overflowX)).toBe("auto")
  }
  await expectNoHorizontalOverflow(page)
}

export async function expectReducedMotion(page: Page): Promise<void> {
  await page.emulateMedia({ reducedMotion: "reduce" })
  await expect.poll(() => page.evaluate(() => matchMedia("(prefers-reduced-motion: reduce)").matches)).toBe(true)
  const durations = await page.locator(".reader-article-plane button").first().evaluate((element) => {
    const style = getComputedStyle(element)
    return [...style.animationDuration.split(","), ...style.transitionDuration.split(",")]
  })
  expect(durations.every((value) => milliseconds(value.trim()) <= 0.01)).toBe(true)
}

function milliseconds(value: string): number {
  if (value.endsWith("ms")) return Number.parseFloat(value)
  if (value.endsWith("s")) return Number.parseFloat(value) * 1000
  return Number.parseFloat(value)
}
