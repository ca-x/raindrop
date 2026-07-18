import type { ListEntriesOptions } from "../api/entries"
import type { Category } from "../api/organization.generated"
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
): Promise<Category[]> {
  return (await api.listCategories(signal)).items
}

export async function loadAllSubscriptions(
  api: ReaderApi,
  signal: AbortSignal,
  current: () => boolean,
): Promise<Subscription[]> {
  const subscriptions: Subscription[] = []
  let cursor: string | undefined
  do {
    const page = await api.listSubscriptions({ cursor, signal })
    if (!current()) return subscriptions
    subscriptions.push(...page.items)
    cursor = page.nextCursor ?? undefined
  } while (cursor !== undefined)
  return subscriptions
}
