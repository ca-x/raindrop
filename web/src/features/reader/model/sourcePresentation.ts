import type { ReaderState } from "./types"

export function selectedSourceLabel(
  state: ReaderState,
  translate: (id: string) => string,
): string {
  const source = state.selectedSource
  if (source.kind === "smart") {
    return translate(
      source.state === "UNREAD"
        ? "reader.unread"
        : source.state === "ALL"
          ? "reader.all"
          : "reader.starred",
    )
  }
  if (source.kind === "category") {
    return state.categoriesById[source.categoryId]?.title ?? translate("reader.categories")
  }
  return (
    state.subscriptionOrder
      .map((id) => state.subscriptionsById[id])
      .find((subscription) => subscription.feedId === source.feedId)?.title ??
    translate("reader.subscriptions")
  )
}
