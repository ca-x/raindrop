import type { Category } from "../api/organization.generated"
import type { ReaderSubscription } from "../model/types"

export interface SubscriptionGroup {
  category: Category | null
  subscriptions: ReaderSubscription[]
  unreadCount: number
}

export function groupSubscriptions(
  categories: Category[],
  subscriptions: ReaderSubscription[],
): { categorized: SubscriptionGroup[]; uncategorized: SubscriptionGroup } {
  const groups = new Map(
    categories.map((category) => [
      category.categoryId,
      { category, subscriptions: [], unreadCount: 0 } satisfies SubscriptionGroup,
    ]),
  )
  const uncategorized: SubscriptionGroup = {
    category: null,
    subscriptions: [],
    unreadCount: 0,
  }

  for (const subscription of subscriptions) {
    const group = subscription.categoryId
      ? groups.get(subscription.categoryId) ?? uncategorized
      : uncategorized
    group.subscriptions.push(subscription)
    group.unreadCount += subscription.unreadCount
  }

  return { categorized: [...groups.values()], uncategorized }
}
