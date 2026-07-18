import type { Subscription } from "../api/subscription.generated"
import type { ReaderSource, ReaderState } from "./types"

export type UnreadSourceDirection = 1 | -1

export function adjacentUnreadSource(
  state: ReaderState,
  direction: UnreadSourceDirection,
): ReaderSource | null {
  const ordered = renderedSubscriptions(state)
  const start = startIndex(state, ordered, direction)
  for (
    let index = start + direction;
    index >= 0 && index < ordered.length;
    index += direction
  ) {
    const subscription = ordered[index]
    if (subscription.unreadCount > 0) {
      return { kind: "feed", feedId: subscription.feedId }
    }
  }
  const fallback: ReaderSource = { kind: "smart", state: "UNREAD" }
  return state.selectedSource.kind === "smart" && state.selectedSource.state === "UNREAD"
    ? null
    : fallback
}

function renderedSubscriptions(state: ReaderState): Subscription[] {
  const subscriptions = state.subscriptionOrder.map((id) => state.subscriptionsById[id])
  return [
    ...state.categoryOrder.flatMap((categoryId) =>
      subscriptions.filter((subscription) => subscription.categoryId === categoryId),
    ),
    ...subscriptions.filter((subscription) => subscription.categoryId === null),
  ]
}

function startIndex(
  state: ReaderState,
  ordered: Subscription[],
  direction: UnreadSourceDirection,
): number {
  const selected = state.selectedSource
  if (selected.kind === "feed") {
    const index = ordered.findIndex((subscription) => subscription.feedId === selected.feedId)
    return index === -1 ? (direction === 1 ? -1 : ordered.length) : index
  }
  if (selected.kind === "category") {
    const indexes = ordered.flatMap((subscription, index) =>
      subscription.categoryId === selected.categoryId ? [index] : [],
    )
    if (indexes.length > 0) {
      return direction === 1 ? indexes[0] - 1 : indexes[indexes.length - 1] + 1
    }
    const categoryPosition = state.categoryOrder.indexOf(selected.categoryId)
    if (categoryPosition >= 0) {
      const boundary = ordered.findIndex((subscription) => {
        const position = subscription.categoryId
          ? state.categoryOrder.indexOf(subscription.categoryId)
          : state.categoryOrder.length
        return position > categoryPosition
      })
      const nextBoundary = boundary === -1 ? ordered.length : boundary
      return direction === 1 ? nextBoundary - 1 : nextBoundary
    }
  }
  return direction === 1 ? -1 : ordered.length
}
