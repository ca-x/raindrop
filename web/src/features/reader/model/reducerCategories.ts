import type { Category } from "../api/organization.generated"
import { sourceKey, type ReaderState } from "./types"

export function upsertCategory(state: ReaderState, category: Category): ReaderState {
  const categoriesById = {
    ...state.categoriesById,
    [category.categoryId]: category,
  }
  return {
    ...state,
    categoriesById,
    categoryOrder: sortedCategoryIds(categoriesById),
    errors: { ...state.errors, mutation: null },
  }
}

export function deleteCategoryState(
  state: ReaderState,
  categoryId: string,
): ReaderState {
  if (!(categoryId in state.categoriesById)) return state
  const categoriesById = { ...state.categoriesById }
  delete categoriesById[categoryId]

  const subscriptionsById = Object.fromEntries(
    Object.entries(state.subscriptionsById).map(([subscriptionId, subscription]) => [
      subscriptionId,
      subscription.categoryId === categoryId
        ? { ...subscription, categoryId: null }
        : subscription,
    ]),
  )
  const key = sourceKey({ kind: "category", categoryId })
  const queueBySourceKey = { ...state.queueBySourceKey }
  const pendingNewEntriesBySource = { ...state.pendingNewEntriesBySource }
  const pendingNewEntryCountBySource = { ...state.pendingNewEntryCountBySource }
  delete queueBySourceKey[key]
  delete pendingNewEntriesBySource[key]
  delete pendingNewEntryCountBySource[key]

  const selectedDeletedCategory =
    state.selectedSource.kind === "category" &&
    state.selectedSource.categoryId === categoryId
  return {
    ...state,
    categoriesById,
    categoryOrder: state.categoryOrder.filter((id) => id !== categoryId),
    subscriptionsById,
    queueBySourceKey,
    pendingNewEntriesBySource,
    pendingNewEntryCountBySource,
    selectedSource: selectedDeletedCategory
      ? { kind: "smart", state: "UNREAD" }
      : state.selectedSource,
    selectedEntryId: selectedDeletedCategory ? null : state.selectedEntryId,
    errors: { ...state.errors, mutation: null },
  }
}

export function sortedCategoryIds(
  categoriesById: ReaderState["categoriesById"],
): string[] {
  return Object.values(categoriesById)
    .sort(
      (left, right) =>
        left.position - right.position || left.categoryId.localeCompare(right.categoryId),
    )
    .map((category) => category.categoryId)
}
