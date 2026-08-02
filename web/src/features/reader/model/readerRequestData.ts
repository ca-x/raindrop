import type { ListEntriesOptions } from "../api/entries"
import type { CategoryList } from "../api/organization.generated"
import type { Subscription } from "../api/subscription.generated"
import type { ReaderApi } from "./controllerApi"
import { sourceKey, type ReaderSource } from "./types"

export function entryListOptions(
  source: ReaderSource,
  searchQuery: string,
): ListEntriesOptions {
  switch (source.kind) {
    case "smart":
      return { state: source.state }
    case "feed":
      return {
        feedId: source.feedId,
        state: "ALL",
        search: searchQuery || undefined,
      }
    case "category":
      return { categoryId: source.categoryId, state: "ALL" }
  }
}

export function sameSource(left: ReaderSource, right: ReaderSource): boolean {
  return sourceKey(left) === sourceKey(right)
}

export async function loadCategories(
  api: ReaderApi,
  signal: AbortSignal,
  expectedOwnerUserId?: string,
): Promise<CategoryList> {
  const categories = await api.listCategories(signal)
  if (
    expectedOwnerUserId !== undefined &&
    categories.ownerUserId !== expectedOwnerUserId
  ) {
    throw new ReaderResponseOwnerMismatchError()
  }
  return categories
}

export interface LoadedSubscriptions {
  ownerUserId: string
  items: Subscription[]
}

export class ReaderResponseOwnerMismatchError extends Error {
  constructor() {
    super("Reader response owner does not match the active session")
    this.name = "ReaderResponseOwnerMismatchError"
  }
}

export async function loadAllSubscriptions(
  api: ReaderApi,
  signal: AbortSignal,
  current: () => boolean,
  expectedOwnerUserId?: string,
): Promise<LoadedSubscriptions> {
  const subscriptions: Subscription[] = []
  let ownerUserId: string | null = null
  let cursor: string | undefined
  do {
    const page = await api.listSubscriptions({ cursor, signal })
    if (!current()) {
      return { ownerUserId: ownerUserId ?? page.ownerUserId, items: subscriptions }
    }
    if (
      (expectedOwnerUserId !== undefined &&
        expectedOwnerUserId !== page.ownerUserId) ||
      (ownerUserId !== null && ownerUserId !== page.ownerUserId)
    ) {
      throw new ReaderResponseOwnerMismatchError()
    }
    ownerUserId = page.ownerUserId
    subscriptions.push(...page.items)
    cursor = page.nextCursor ?? undefined
  } while (cursor !== undefined)
  return { ownerUserId: ownerUserId!, items: subscriptions }
}
