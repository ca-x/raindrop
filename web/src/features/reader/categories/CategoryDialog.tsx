import { AlertDialog } from "@astryxdesign/core/AlertDialog"
import { Button } from "@astryxdesign/core/Button"
import { Dialog, DialogHeader } from "@astryxdesign/core/Dialog"
import { Layout, LayoutFooter } from "@astryxdesign/core/Layout"
import { useLingui } from "@lingui/react"
import { useEffect, useState, type FormEvent } from "react"

import {
  isCreateCategoryRequest,
  isUpdateCategoryRequest,
  type Category,
} from "../api/organization.generated"
import type { Subscription } from "../api/subscription.generated"
import {
  CategoryDialogContent,
  uncategorizedValue,
} from "./CategoryDialogContent"

interface CategoryDialogProps {
  isOpen: boolean
  categories: Category[]
  subscriptions: Subscription[]
  selectedSubscription?: Subscription
  mutationError: string | null
  onOpenChange: (isOpen: boolean) => void
  onClearError: () => void
  onCreate: (title: string) => Promise<boolean>
  onUpdate: (categoryId: string, title: string) => Promise<boolean>
  onDelete: (categoryId: string) => Promise<boolean>
  onAssign: (subscriptionId: string, categoryId: string | null) => Promise<boolean>
}

export function CategoryDialog(props: CategoryDialogProps) {
  const { i18n } = useLingui()
  const [newTitle, setNewTitle] = useState("")
  const [selectedCategoryId, setSelectedCategoryId] = useState<string | null>(null)
  const [editTitle, setEditTitle] = useState("")
  const [createValidationError, setCreateValidationError] = useState<string | null>(null)
  const [editValidationError, setEditValidationError] = useState<string | null>(null)
  const [isCreating, setIsCreating] = useState(false)
  const [isSaving, setIsSaving] = useState(false)
  const [isDeleteOpen, setIsDeleteOpen] = useState(false)
  const [isDeleting, setIsDeleting] = useState(false)
  const [isAssigning, setIsAssigning] = useState(false)
  const selectedCategory = props.categories.find(
    (category) => category.categoryId === selectedCategoryId,
  )
  const serverAssignment =
    props.selectedSubscription?.categoryId ?? uncategorizedValue
  const [assignment, setAssignment] = useState(serverAssignment)

  useEffect(() => {
    if (!props.isOpen) return
    const nextSelected =
      props.categories.find((category) => category.categoryId === selectedCategoryId) ??
      props.categories[0]
    setSelectedCategoryId(nextSelected?.categoryId ?? null)
    setEditTitle(nextSelected?.title ?? "")
  }, [props.categories, props.isOpen, selectedCategoryId])

  useEffect(() => setAssignment(serverAssignment), [serverAssignment])

  const affectedSubscriptions = selectedCategory
    ? props.subscriptions.filter(
        (subscription) => subscription.categoryId === selectedCategory.categoryId,
      ).length
    : 0
  const isBusy = isCreating || isSaving || isDeleting || isAssigning

  const close = () => {
    if (isBusy) return
    setCreateValidationError(null)
    setEditValidationError(null)
    props.onClearError()
    props.onOpenChange(false)
  }

  const create = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    const request = { title: newTitle.trim() }
    if (!isCreateCategoryRequest(request)) {
      setCreateValidationError(i18n._("reader.categoryTitleInvalid"))
      return
    }
    setCreateValidationError(null)
    props.onClearError()
    setIsCreating(true)
    const created = await props.onCreate(request.title)
    setIsCreating(false)
    if (created) setNewTitle("")
  }

  const save = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (!selectedCategory) return
    const request = { title: editTitle.trim() }
    if (!isUpdateCategoryRequest(request)) {
      setEditValidationError(i18n._("reader.categoryTitleInvalid"))
      return
    }
    setEditValidationError(null)
    props.onClearError()
    setIsSaving(true)
    await props.onUpdate(selectedCategory.categoryId, request.title)
    setIsSaving(false)
  }

  const assign = async (value: string) => {
    if (!props.selectedSubscription) return
    const previous = assignment
    setAssignment(value)
    setEditValidationError(null)
    props.onClearError()
    setIsAssigning(true)
    const assigned = await props.onAssign(
      props.selectedSubscription.subscriptionId,
      value === uncategorizedValue ? null : value,
    )
    setIsAssigning(false)
    if (!assigned) setAssignment(previous)
  }

  const deleteSelected = async () => {
    if (!selectedCategory) return
    props.onClearError()
    setIsDeleting(true)
    const deleted = await props.onDelete(selectedCategory.categoryId)
    setIsDeleting(false)
    if (!deleted) {
      setIsDeleteOpen(false)
      return
    }
    setIsDeleteOpen(false)
    setSelectedCategoryId(null)
    setEditTitle("")
  }

  return (
    <>
      <Dialog
        isOpen={props.isOpen && !isDeleteOpen}
        aria-label={i18n._("reader.manageCategories")}
        onOpenChange={(open) => {
          if (!open) close()
        }}
        purpose="form"
        width="min(720px, calc(100vw - 24px))"
        maxHeight="min(760px, calc(100dvh - 24px))"
        className="reader-category-dialog"
      >
        <Layout
          height="auto"
          padding={0}
          header={
            <DialogHeader
              title={i18n._("reader.manageCategories")}
              subtitle={i18n._("reader.manageCategoriesDescription")}
              hasDivider
            />
          }
          content={
            <CategoryDialogContent
              categories={props.categories}
              subscriptions={props.subscriptions}
              selectedCategory={selectedCategory}
              selectedCategoryId={selectedCategoryId}
              selectedSubscription={props.selectedSubscription}
              newTitle={newTitle}
              editTitle={editTitle}
              assignment={assignment}
              mutationError={props.mutationError}
              createValidationError={createValidationError}
              editValidationError={editValidationError}
              isCreating={isCreating}
              isSaving={isSaving}
              isAssigning={isAssigning}
              onCreate={create}
              onSave={save}
              onNewTitleChange={(value) => {
                setNewTitle(value)
                setCreateValidationError(null)
                if (props.mutationError) props.onClearError()
              }}
              onEditTitleChange={(value) => {
                setEditTitle(value)
                setEditValidationError(null)
                if (props.mutationError) props.onClearError()
              }}
              onSelectCategory={(category) => {
                setSelectedCategoryId(category.categoryId)
                setEditTitle(category.title)
                setEditValidationError(null)
                props.onClearError()
              }}
              onDeleteRequest={() => setIsDeleteOpen(true)}
              onAssignmentChange={(value) => void assign(value)}
            />
          }
          footer={
            <LayoutFooter hasDivider padding={3}>
              <div className="reader-dialog-actions">
                <Button
                  label={i18n._("common.close")}
                  onClick={close}
                  variant="secondary"
                />
              </div>
            </LayoutFooter>
          }
        />
      </Dialog>

      <AlertDialog
        isOpen={isDeleteOpen}
        onOpenChange={(open) => {
          if (!isDeleting) setIsDeleteOpen(open)
        }}
        title={i18n._("reader.deleteCategoryTitle")}
        description={i18n._("reader.deleteCategoryDescription", {
          title: selectedCategory?.title ?? "",
          count: affectedSubscriptions,
        })}
        actionLabel={i18n._("reader.deleteCategory")}
        cancelLabel={i18n._("common.cancel")}
        isActionLoading={isDeleting}
        onAction={() => void deleteSelected()}
        width="min(440px, calc(100vw - 24px))"
      />
    </>
  )
}
