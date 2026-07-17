import type {
  EntryDetailResponse,
  EntryListItemResponse,
} from "../api/reader.generated"
import type { Subscription } from "../api/subscription.generated"

export const feedId = "00000000-0000-4000-8000-000000000101"
export const subscriptionId = "00000000-0000-4000-8000-000000000201"
export const entryId = "00000000-0000-4000-8000-000000000301"

export function makeSubscription(
  overrides: Partial<Subscription> = {},
): Subscription {
  return {
    subscriptionId,
    feedId,
    title: "Example Feed",
    siteUrl: "https://example.com/",
    unreadCount: 3,
    refresh: null,
    ...overrides,
  }
}

export function makeEntry(
  overrides: Partial<EntryListItemResponse> = {},
): EntryListItemResponse {
  return {
    entryId,
    feedId,
    feedTitle: "Example Feed",
    siteUrl: "https://example.com/",
    title: "Entry title",
    author: "Reader",
    summary: "Summary",
    canonicalUrl: "https://example.com/article",
    publishedAtUs: 1_784_246_400_000_000,
    sortAtUs: 1_784_246_400_000_000,
    isRead: false,
    isStarred: false,
    ...overrides,
  }
}

export function makeDetail(
  overrides: Partial<EntryDetailResponse> = {},
): EntryDetailResponse {
  return {
    ...makeEntry(),
    contentHtml: "<p>Safe article</p>",
    inertImages: [],
    enclosures: [],
    ...overrides,
  }
}
