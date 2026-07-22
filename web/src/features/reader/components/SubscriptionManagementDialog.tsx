import { AlertDialog } from "@astryxdesign/core/AlertDialog"
import { Banner } from "@astryxdesign/core/Banner"
import { Button } from "@astryxdesign/core/Button"
import { Dialog, DialogHeader } from "@astryxdesign/core/Dialog"
import { FormLayout } from "@astryxdesign/core/FormLayout"
import { Layout, LayoutContent, LayoutFooter } from "@astryxdesign/core/Layout"
import { List, ListItem } from "@astryxdesign/core/List"
import { Selector } from "@astryxdesign/core/Selector"
import { Tab, TabList } from "@astryxdesign/core/TabList"
import { TextInput } from "@astryxdesign/core/TextInput"
import { useLingui } from "@lingui/react"
import { useEffect, useState, type FormEvent } from "react"

import { OpmlTransferPanel } from "../../opml/components/OpmlTransferPanel"
import {
  isCreateCategoryRequest,
  isUpdateCategoryRequest,
  type Category,
} from "../api/organization.generated"
import {
  isCreateSubscriptionRequest,
  type CreateSubscriptionResponse,
  type Subscription,
  type UpdateSubscriptionRequest,
} from "../api/subscription.generated"

const uncategorizedValue = "__uncategorized__"

type ManagementTab = "subscriptions" | "categories" | "opml"

interface PendingSubscriptionSetup {
  subscriptionId: string
  created: boolean
  titleOverride: string
  categoryId: string
}

interface SubscriptionManagementDialogProps {
  isOpen: boolean
  subscriptions: Subscription[]
  categories: Category[]
  mutationError: string | null
  csrfToken: string
  onOpenChange: (isOpen: boolean) => void
  onClearError: () => void
  onAdd: (url: string) => Promise<CreateSubscriptionResponse | null>
  onUpdate: (
    subscriptionId: string,
    request: UpdateSubscriptionRequest,
  ) => Promise<boolean>
  onDelete: (subscriptionId: string) => Promise<boolean>
  onCreateCategory: (title: string) => Promise<boolean>
  onUpdateCategory: (categoryId: string, title: string) => Promise<boolean>
  onDeleteCategory: (categoryId: string) => Promise<boolean>
  onSubscriptionsChanged: () => Promise<void> | void
}

export function SubscriptionManagementDialog(
  props: SubscriptionManagementDialogProps,
) {
  const { i18n } = useLingui()
  const [activeTab, setActiveTab] = useState<ManagementTab>("subscriptions")
  const [pendingSubscription, setPendingSubscription] =
    useState<PendingSubscriptionSetup | null>(null)

  useEffect(() => {
    if (props.isOpen) {
      setActiveTab("subscriptions")
      setPendingSubscription(null)
    }
  }, [props.isOpen])

  const close = () => {
    props.onClearError()
    props.onOpenChange(false)
  }

  return (
    <Dialog
      isOpen={props.isOpen}
      aria-label={i18n._("reader.manageSubscriptions")}
      onOpenChange={(open) => {
        if (!open) close()
      }}
      purpose="form"
      width="min(760px, calc(100vw - 24px))"
      maxHeight="min(780px, calc(100dvh - 24px))"
      className="reader-subscription-management-dialog"
    >
      <Layout
        height="fill"
        padding={0}
        header={(
          <DialogHeader
            title={i18n._("reader.manageSubscriptions")}
            subtitle={i18n._("reader.manageSubscriptionsDescription")}
            hasDivider
            className="reader-dialog-header"
          />
        )}
        content={(
          <LayoutContent padding={0} className="reader-management-content">
            <div className="reader-management-tabs">
              <TabList
                value={activeTab}
                onChange={(value) => setActiveTab(value as ManagementTab)}
                layout="fill"
                hasDivider
              >
                <Tab value="subscriptions" label={i18n._("reader.subscriptions")} />
                <Tab value="categories" label={i18n._("reader.addCategoryTab")} />
                <Tab value="opml" label="OPML" />
              </TabList>
            </div>
            <div
              key={activeTab}
              className="reader-management-panel reader-panel-transition"
            >
              {activeTab === "subscriptions" ? (
                <SubscriptionPanel
                  {...props}
                  pendingSubscription={pendingSubscription}
                  onPendingSubscriptionChange={setPendingSubscription}
                  onComplete={close}
                />
              ) : activeTab === "categories" ? (
                <CategoryPanel {...props} />
              ) : (
                <div role="tabpanel" aria-label="OPML">
                  <OpmlTransferPanel
                    csrfToken={props.csrfToken}
                    onImported={props.onSubscriptionsChanged}
                  />
                </div>
              )}
            </div>
          </LayoutContent>
        )}
        footer={(
          <LayoutFooter hasDivider padding={3}>
            <div className="reader-dialog-actions">
              <Button label={i18n._("common.close")} onClick={close} variant="secondary" />
            </div>
          </LayoutFooter>
        )}
      />
    </Dialog>
  )
}

function SubscriptionPanel(
  props: SubscriptionManagementDialogProps & {
    pendingSubscription: PendingSubscriptionSetup | null
    onPendingSubscriptionChange: (
      pending: PendingSubscriptionSetup | null,
    ) => void
    onComplete: () => void
  },
) {
  const { i18n } = useLingui()
  const [url, setUrl] = useState("")
  const [validationError, setValidationError] = useState<string | null>(null)
  const [isAdding, setIsAdding] = useState(false)
  const [isOrganizing, setIsOrganizing] = useState(false)
  const [isDiscarding, setIsDiscarding] = useState(false)
  const addedSubscription = props.pendingSubscription
    ? props.subscriptions.find(
        (candidate) =>
          candidate.subscriptionId === props.pendingSubscription?.subscriptionId,
      )
    : undefined

  const categoryOptions = [
    { value: uncategorizedValue, label: i18n._("reader.uncategorized") },
    ...props.categories.map((category) => ({
      value: category.categoryId,
      label: category.title,
    })),
  ]

  const add = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    const value = url.trim()
    if (!isCreateSubscriptionRequest({ url: value })) {
      setValidationError(i18n._("reader.feedUrlInvalid"))
      return
    }
    setValidationError(null)
    props.onClearError()
    setIsAdding(true)
    const result = await props.onAdd(value)
    setIsAdding(false)
    if (!result) return
    const { subscription } = result
    props.onPendingSubscriptionChange({
      subscriptionId: subscription.subscriptionId,
      created: result.created,
      titleOverride: subscription.titleOverride ?? "",
      categoryId: subscription.categoryId ?? uncategorizedValue,
    })
  }

  const finishAdd = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (!addedSubscription || !props.pendingSubscription) return
    const request: Partial<UpdateSubscriptionRequest> = {}
    const nextTitle = props.pendingSubscription.titleOverride.trim() || null
    const nextCategory =
      props.pendingSubscription.categoryId === uncategorizedValue
        ? null
        : props.pendingSubscription.categoryId
    if (nextTitle !== addedSubscription.titleOverride) request.titleOverride = nextTitle
    if (nextCategory !== addedSubscription.categoryId) request.categoryId = nextCategory
    setIsOrganizing(true)
    const saved = Object.keys(request).length === 0
      ? true
      : await props.onUpdate(
          addedSubscription.subscriptionId,
          request as UpdateSubscriptionRequest,
        )
    setIsOrganizing(false)
    if (!saved) return
    props.onComplete()
  }

  const returnToFeedUrl = async () => {
    if (!props.pendingSubscription?.created) {
      props.onPendingSubscriptionChange(null)
      return
    }
    if (!addedSubscription) return
    setIsDiscarding(true)
    const deleted = await props.onDelete(addedSubscription.subscriptionId)
    setIsDiscarding(false)
    if (deleted) props.onPendingSubscriptionChange(null)
  }

  return (
    <div role="tabpanel" aria-label={i18n._("reader.subscriptions")} className="reader-management-stack">
      {props.mutationError ? (
        <Banner
          container="section"
          status="error"
          title={i18n._("reader.feedManagementError")}
          description={props.mutationError}
        />
      ) : null}

      <section className="reader-management-section" aria-labelledby="reader-add-feed-heading">
        <div>
          <div id="reader-add-feed-heading" className="reader-preference-label">
            {i18n._("reader.addSubscription")}
          </div>
          <div className="reader-preference-description">
            {props.pendingSubscription
              ? i18n._(
                  props.pendingSubscription.created
                    ? "reader.organizeSubscriptionDescription"
                    : "reader.organizeExistingSubscriptionDescription",
                )
              : i18n._("reader.addSubscriptionDescription")}
          </div>
        </div>
        {props.pendingSubscription ? addedSubscription ? (
          <form onSubmit={finishAdd} noValidate>
            <FormLayout>
              <TextInput
                label={i18n._("reader.feedUrl")}
                value={addedSubscription.feedUrl}
                isDisabled
                width="100%"
              />
              <TextInput
                label={i18n._("reader.feedCustomTitle")}
                value={props.pendingSubscription.titleOverride}
                onChange={(titleOverride) =>
                  props.onPendingSubscriptionChange({
                    ...props.pendingSubscription!,
                    titleOverride,
                  })
                }
                placeholder={addedSubscription.title}
                width="100%"
              />
              <Selector
                label={i18n._("reader.feedCategory")}
                options={categoryOptions}
                value={props.pendingSubscription.categoryId}
                onChange={(categoryId) =>
                  props.onPendingSubscriptionChange({
                    ...props.pendingSubscription!,
                    categoryId,
                  })
                }
                hasSearch={props.categories.length > 8}
                placement="below"
                width="100%"
              />
              <div className="reader-category-form-actions">
                <Button
                  label={i18n._("reader.backToFeedUrl")}
                  onClick={() => void returnToFeedUrl()}
                  isLoading={isDiscarding}
                  isDisabled={isOrganizing}
                  variant="secondary"
                />
                <Button
                  label={i18n._("reader.finishSubscription")}
                  type="submit"
                  isLoading={isOrganizing}
                  isDisabled={isDiscarding}
                  variant="primary"
                />
              </div>
            </FormLayout>
          </form>
        ) : (
          <div role="status" className="reader-preference-description">
            {i18n._("reader.loadingSubscriptions")}
          </div>
        ) : (
          <form onSubmit={add} noValidate>
            <FormLayout>
              <TextInput
                label={i18n._("reader.feedUrl")}
                value={url}
                onChange={(value) => {
                  setUrl(value)
                  setValidationError(null)
                  if (props.mutationError) props.onClearError()
                }}
                placeholder="https://example.com/feed.xml"
                status={validationError ? { type: "error", message: validationError } : undefined}
                isRequired
                width="100%"
              />
              <div className="reader-category-form-actions">
                <Button
                  label={i18n._("reader.continueSubscription")}
                  type="submit"
                  isLoading={isAdding}
                  variant="primary"
                />
              </div>
            </FormLayout>
          </form>
        )}
      </section>

    </div>
  )
}

function CategoryPanel(props: SubscriptionManagementDialogProps) {
  const { i18n } = useLingui()
  const [newTitle, setNewTitle] = useState("")
  const [selectedCategoryId, setSelectedCategoryId] = useState<string | null>(null)
  const [editTitle, setEditTitle] = useState("")
  const [validationError, setValidationError] = useState<string | null>(null)
  const [isCreating, setIsCreating] = useState(false)
  const [isSaving, setIsSaving] = useState(false)
  const [isDeleting, setIsDeleting] = useState(false)
  const [isDeleteOpen, setIsDeleteOpen] = useState(false)
  const selectedCategory = props.categories.find(
    (category) => category.categoryId === selectedCategoryId,
  )

  useEffect(() => {
    if (!props.isOpen) return
    const selected = selectedCategory ?? props.categories[0]
    setSelectedCategoryId(selected?.categoryId ?? null)
    setEditTitle(selected?.title ?? "")
  }, [props.categories, props.isOpen, selectedCategory])

  const create = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    const request = { title: newTitle.trim() }
    if (!isCreateCategoryRequest(request)) {
      setValidationError(i18n._("reader.categoryTitleInvalid"))
      return
    }
    setValidationError(null)
    setIsCreating(true)
    const created = await props.onCreateCategory(request.title)
    setIsCreating(false)
    if (created) setNewTitle("")
  }

  const save = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (!selectedCategory) return
    const request = { title: editTitle.trim() }
    if (!isUpdateCategoryRequest(request)) {
      setValidationError(i18n._("reader.categoryTitleInvalid"))
      return
    }
    setValidationError(null)
    setIsSaving(true)
    await props.onUpdateCategory(selectedCategory.categoryId, request.title)
    setIsSaving(false)
  }

  const deleteCategory = async () => {
    if (!selectedCategory) return
    setIsDeleting(true)
    const deleted = await props.onDeleteCategory(selectedCategory.categoryId)
    setIsDeleting(false)
    setIsDeleteOpen(false)
    if (deleted) setSelectedCategoryId(null)
  }

  const affectedCount = selectedCategory
    ? props.subscriptions.filter(
        (subscription) => subscription.categoryId === selectedCategory.categoryId,
      ).length
    : 0

  return (
    <div role="tabpanel" aria-label={i18n._("reader.addCategoryTab")} className="reader-management-stack">
      {props.mutationError ? (
        <Banner status="error" title={i18n._("reader.categoryMutationError")} description={props.mutationError} />
      ) : null}
      <section className="reader-management-section">
        <form onSubmit={create} noValidate>
          <FormLayout>
            <TextInput
              label={i18n._("reader.newCategory")}
              value={newTitle}
              onChange={(value) => {
                setNewTitle(value)
                setValidationError(null)
              }}
              placeholder={i18n._("reader.categoryNamePlaceholder")}
              status={validationError ? { type: "error", message: validationError } : undefined}
              isRequired
              width="100%"
            />
            <div className="reader-category-form-actions">
              <Button
                label={i18n._("reader.createCategory")}
                type="submit"
                isLoading={isCreating}
                variant="primary"
              />
            </div>
          </FormLayout>
        </form>
      </section>
      <section className="reader-management-section reader-category-editor">
        <List density="compact" hasDividers className="reader-category-management-list">
          {props.categories.length > 0 ? props.categories.map((category) => (
            <ListItem
              key={category.categoryId}
              label={category.title}
              description={i18n._("reader.categoryFeedCount", {
                count: props.subscriptions.filter(
                  (subscription) => subscription.categoryId === category.categoryId,
                ).length,
              })}
              isSelected={selectedCategoryId === category.categoryId}
              onClick={() => {
                setSelectedCategoryId(category.categoryId)
                setEditTitle(category.title)
                setValidationError(null)
              }}
            />
          )) : (
            <ListItem
              label={i18n._("reader.noCategories")}
              description={i18n._("reader.noCategoriesDescription")}
              isDisabled
            />
          )}
        </List>
        <form onSubmit={save} noValidate>
          <FormLayout>
            <TextInput
              label={i18n._("reader.categoryName")}
              value={editTitle}
              onChange={setEditTitle}
              isDisabled={!selectedCategory}
              disabledMessage={i18n._("reader.selectCategoryToEdit")}
              width="100%"
            />
            <div className="reader-category-form-actions">
              <Button
                label={i18n._("reader.deleteCategory")}
                onClick={() => setIsDeleteOpen(true)}
                isDisabled={!selectedCategory}
                variant="destructive"
              />
              <Button
                label={i18n._("reader.saveCategory")}
                type="submit"
                isLoading={isSaving}
                isDisabled={!selectedCategory || editTitle.trim() === selectedCategory.title}
                variant="primary"
              />
            </div>
          </FormLayout>
        </form>
      </section>
      <AlertDialog
        isOpen={isDeleteOpen}
        onOpenChange={(open) => {
          if (!isDeleting) setIsDeleteOpen(open)
        }}
        title={i18n._("reader.deleteCategoryTitle")}
        description={i18n._("reader.deleteCategoryDescription", {
          title: selectedCategory?.title ?? "",
          count: affectedCount,
        })}
        actionLabel={i18n._("reader.deleteCategory")}
        cancelLabel={i18n._("common.cancel")}
        isActionLoading={isDeleting}
        onAction={() => void deleteCategory()}
      />
    </div>
  )
}
