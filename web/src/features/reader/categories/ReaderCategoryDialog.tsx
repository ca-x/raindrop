import type { ReaderSource } from "../model/types"
import type { ReaderController } from "../model/useReaderController"
import { CategoryDialog } from "./CategoryDialog"

interface ReaderCategoryDialogProps {
  controller: ReaderController
  isOpen: boolean
  onOpenChange: (isOpen: boolean) => void
  onSelectSource: (source: ReaderSource) => void
}

export function ReaderCategoryDialog(props: ReaderCategoryDialogProps) {
  if (!props.isOpen) return null

  const categories = props.controller.state.categoryOrder.map(
    (id) => props.controller.state.categoriesById[id],
  )
  const subscriptions = props.controller.state.subscriptionOrder.map(
    (id) => props.controller.state.subscriptionsById[id],
  )
  const selectedSource = props.controller.state.selectedSource
  const selectedSubscription =
    selectedSource.kind === "feed"
      ? subscriptions.find(
          (subscription) => subscription.feedId === selectedSource.feedId,
        )
      : undefined

  return (
    <CategoryDialog
      isOpen={props.isOpen}
      categories={categories}
      subscriptions={subscriptions}
      selectedSubscription={selectedSubscription}
      mutationError={props.controller.state.errors.mutation}
      onOpenChange={props.onOpenChange}
      onClearError={props.controller.clearMutationError}
      onCreate={props.controller.createCategory}
      onUpdate={(categoryId, title) =>
        props.controller.updateCategory(categoryId, { title })
      }
      onDelete={async (categoryId) => {
        const wasActive =
          props.controller.state.selectedSource.kind === "category" &&
          props.controller.state.selectedSource.categoryId === categoryId
        const deleted = await props.controller.deleteCategory(categoryId)
        if (deleted && wasActive) {
          props.onSelectSource({ kind: "smart", state: "UNREAD" })
        }
        return deleted
      }}
      onAssign={(subscriptionId, categoryId) =>
        props.controller.updateSubscription(subscriptionId, { categoryId })
      }
    />
  )
}
