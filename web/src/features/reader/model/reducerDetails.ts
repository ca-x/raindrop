import type {
  EntryDetailResponse,
  EntryListItemResponse,
} from "../api/reader.generated"

export function entryDetailToListItem(
  detail: EntryDetailResponse,
): EntryListItemResponse {
  return {
    entryId: detail.entryId,
    feedId: detail.feedId,
    feedTitle: detail.feedTitle,
    siteUrl: detail.siteUrl,
    title: detail.title,
    author: detail.author,
    summary: detail.summary,
    canonicalUrl: detail.canonicalUrl,
    publishedAtUs: detail.publishedAtUs,
    sortAtUs: detail.sortAtUs,
    isRead: detail.isRead,
    isStarred: detail.isStarred,
  }
}

export function updateDetailsFromEntries(
  detailsById: Record<string, EntryDetailResponse>,
  entries: EntryListItemResponse[],
): Record<string, EntryDetailResponse> {
  let updated = detailsById
  for (const entry of entries) {
    const detail = updated[entry.entryId]
    if (!detail) continue
    if (updated === detailsById) updated = { ...detailsById }
    updated[entry.entryId] = { ...detail, ...entry }
  }
  return updated
}
