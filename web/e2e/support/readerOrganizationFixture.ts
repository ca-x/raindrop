import type { Page, Route } from "@playwright/test"

import type {
  Category,
  CreateCategoryRequest,
  UpdateCategoryRequest,
} from "../../src/features/reader/api/organization.generated"
import type {
  Refresh,
  Subscription,
  UpdateSubscriptionRequest,
} from "../../src/features/reader/api/subscription.generated"

export const readerOrganizationIds = {
  categoryA: "00000000-0000-4000-8000-000000000501",
  categoryB: "00000000-0000-4000-8000-000000000502",
  otherUserCategory: "00000000-0000-4000-8000-000000000598",
  otherUserSubscription: "00000000-0000-4000-8000-000000000298",
} as const

interface CategoryCall {
  method: "POST" | "PATCH" | "DELETE"
  categoryId: string | null
  csrf: string | undefined
}

interface SubscriptionPatchCall {
  subscriptionId: string
  body: UpdateSubscriptionRequest
  csrf: string | undefined
}

export interface ReaderOrganizationFixture {
  categories: Category[]
  subscriptions: Subscription[]
  categoryCalls: CategoryCall[]
  subscriptionPatches: SubscriptionPatchCall[]
  feedIdsForCategory: (categoryId: string) => Set<string>
}

interface ReaderOrganizationOptions {
  feedA: string
  feedB: string
  subscriptionA: string
  subscriptionB: string
}

export async function installReaderOrganizationFixture(
  page: Page,
  options: ReaderOrganizationOptions,
): Promise<ReaderOrganizationFixture> {
  const categories: Category[] = [
    { categoryId: readerOrganizationIds.categoryA, title: "Engineering", position: 1024 },
  ]
  const subscriptions = createSubscriptions(options)
  const categoryCalls: CategoryCall[] = []
  const subscriptionPatches: SubscriptionPatchCall[] = []

  await page.route("**/api/v1/categories**", async (route) => {
    await handleCategories(route, categories, subscriptions, categoryCalls)
  })
  await page.route("**/api/v1/subscriptions**", async (route) => {
    await handleSubscriptions(route, categories, subscriptions, subscriptionPatches)
  })

  return {
    categories,
    subscriptions,
    categoryCalls,
    subscriptionPatches,
    feedIdsForCategory: (categoryId) =>
      new Set(
        subscriptions
          .filter((subscription) => subscription.categoryId === categoryId)
          .map((subscription) => subscription.feedId),
      ),
  }
}

async function handleCategories(
  route: Route,
  categories: Category[],
  subscriptions: Subscription[],
  calls: CategoryCall[],
): Promise<void> {
  const request = route.request()
  const url = new URL(request.url())
  const method = request.method()
  if (url.pathname === "/api/v1/categories" && method === "GET") {
    await json(route, { items: categories })
    return
  }
  if (url.pathname === "/api/v1/categories" && method === "POST") {
    const body = request.postDataJSON() as CreateCategoryRequest
    const category: Category = {
      categoryId: readerOrganizationIds.categoryB,
      title: body.title,
      position: Math.max(0, ...categories.map((item) => item.position)) + 1024,
    }
    categories.push(category)
    calls.push({ method, categoryId: null, csrf: requireCsrf(request.headers()) })
    await json(route, category, 201)
    return
  }

  const match = /^\/api\/v1\/categories\/([^/]+)$/u.exec(url.pathname)
  if (!match) throw new Error(`unexpected Category request: ${method} ${url.pathname}`)
  if (method !== "PATCH" && method !== "DELETE") {
    throw new Error(`unexpected Category request: ${method} ${url.pathname}`)
  }
  const csrf = requireCsrf(request.headers())
  const categoryId = decodeURIComponent(match[1])
  const index = categories.findIndex((category) => category.categoryId === categoryId)
  if (index < 0 || categoryId === readerOrganizationIds.otherUserCategory) {
    await notFound(route)
    return
  }
  if (method === "PATCH") {
    const body = request.postDataJSON() as UpdateCategoryRequest
    categories[index] = { ...categories[index], ...body }
    calls.push({ method, categoryId, csrf })
    await json(route, categories[index])
    return
  }
  if (method === "DELETE") {
    categories.splice(index, 1)
    for (const subscription of subscriptions) {
      if (subscription.categoryId === categoryId) subscription.categoryId = null
    }
    calls.push({ method, categoryId, csrf })
    await route.fulfill({ status: 204, body: "" })
    return
  }
  throw new Error(`unexpected Category request: ${method} ${url.pathname}`)
}

async function handleSubscriptions(
  route: Route,
  categories: Category[],
  subscriptions: Subscription[],
  patches: SubscriptionPatchCall[],
): Promise<void> {
  const request = route.request()
  const url = new URL(request.url())
  const method = request.method()
  if (url.pathname === "/api/v1/subscriptions" && method === "GET") {
    await json(route, { items: subscriptions, nextCursor: null })
    return
  }
  const refreshMatch = /^\/api\/v1\/subscriptions\/([^/]+)\/refresh$/u.exec(url.pathname)
  if (refreshMatch && method === "POST") {
    requireCsrf(request.headers())
    await json(route, pendingRefresh())
    return
  }
  const match = /^\/api\/v1\/subscriptions\/([^/]+)$/u.exec(url.pathname)
  if (!match || method !== "PATCH") {
    throw new Error(`unexpected Subscription request: ${method} ${url.pathname}`)
  }
  const subscriptionId = decodeURIComponent(match[1])
  const csrf = requireCsrf(request.headers())
  const subscription = subscriptions.find((item) => item.subscriptionId === subscriptionId)
  if (!subscription || subscriptionId === readerOrganizationIds.otherUserSubscription) {
    await notFound(route)
    return
  }
  const body = request.postDataJSON() as UpdateSubscriptionRequest
  if (
    body.categoryId !== undefined &&
    body.categoryId !== null &&
    !categories.some((category) => category.categoryId === body.categoryId)
  ) {
    await notFound(route)
    return
  }
  if (body.categoryId !== undefined) subscription.categoryId = body.categoryId
  if (body.titleOverride !== undefined) subscription.titleOverride = body.titleOverride
  if (body.position !== undefined) subscription.position = body.position
  patches.push({ subscriptionId, body, csrf })
  await json(route, subscription)
}

function createSubscriptions(options: ReaderOrganizationOptions): Subscription[] {
  return [
    subscription(options.subscriptionA, options.feedA, "Quiet Web", "https://quiet.example/", null),
    subscription(
      options.subscriptionB,
      options.feedB,
      "Rust Dispatch",
      "https://rust.example/",
      readerOrganizationIds.categoryA,
    ),
  ]
}

function subscription(
  subscriptionId: string,
  feedId: string,
  title: string,
  siteUrl: string,
  categoryId: string | null,
): Subscription {
  return {
    subscriptionId,
    feedId,
    categoryId,
    titleOverride: null,
    position: 0,
    title,
    siteUrl,
    unreadCount: 6,
    refresh: null,
  }
}

function pendingRefresh(): Refresh {
  return {
    operationId: "00000000-0000-4000-8000-000000000401",
    state: "PENDING",
    pendingState: "QUEUED",
    newCount: 0,
    updatedCount: 0,
    droppedCount: 0,
    entryIssues: [],
    generation: null,
    errorCode: null,
    retryAt: null,
    lastSuccessAt: null,
    queuedAt: "2026-07-18T00:00:00.000000Z",
    startedAt: null,
    completedAt: null,
  }
}

function requireCsrf(headers: Record<string, string>): string {
  const csrf = headers["x-csrf-token"]
  if (!csrf) throw new Error("organization mutation omitted CSRF")
  return csrf
}

async function notFound(route: Route): Promise<void> {
  await json(route, {
    error: {
      code: "NOT_FOUND",
      message: "Resource not found",
      requestId: "00000000-0000-4000-8000-000000000599",
    },
  }, 404)
}

async function json(route: Route, body: unknown, status = 200): Promise<void> {
  await route.fulfill({
    status,
    contentType: "application/json",
    body: JSON.stringify(body),
  })
}
