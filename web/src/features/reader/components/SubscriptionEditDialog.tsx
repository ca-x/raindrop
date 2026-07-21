import { AlertDialog } from "@astryxdesign/core/AlertDialog"
import { Banner } from "@astryxdesign/core/Banner"
import { Button } from "@astryxdesign/core/Button"
import { Dialog, DialogHeader } from "@astryxdesign/core/Dialog"
import { FormLayout } from "@astryxdesign/core/FormLayout"
import { Layout, LayoutContent, LayoutFooter } from "@astryxdesign/core/Layout"
import { Selector } from "@astryxdesign/core/Selector"
import { TextInput } from "@astryxdesign/core/TextInput"
import { useLingui } from "@lingui/react"
import { useEffect, useState, type FormEvent } from "react"

import type { UserPreferencesLinkOpenMode } from "../../preferences/api/preferences.generated"
import type { Category } from "../api/organization.generated"
import type {
  Subscription,
  UpdateSubscriptionRequest,
} from "../api/subscription.generated"

const uncategorizedValue = "__uncategorized__"

interface SubscriptionEditDialogProps {
  isOpen: boolean
  subscription?: Subscription
  categories: Category[]
  mutationError: string | null
  linkOpenMode: UserPreferencesLinkOpenMode
  onOpenChange: (isOpen: boolean) => void
  onClearError: () => void
  onUpdate: (
    subscriptionId: string,
    request: UpdateSubscriptionRequest,
  ) => Promise<boolean>
  onDelete: (subscriptionId: string) => Promise<boolean>
}

export function SubscriptionEditDialog(props: SubscriptionEditDialogProps) {
  const { i18n } = useLingui()
  const [titleOverride, setTitleOverride] = useState("")
  const [categoryId, setCategoryId] = useState(uncategorizedValue)
  const [isSaving, setIsSaving] = useState(false)
  const [isAssigning, setIsAssigning] = useState(false)
  const [isDeleteOpen, setIsDeleteOpen] = useState(false)
  const [isDeleting, setIsDeleting] = useState(false)
  const subscription = props.subscription

  useEffect(() => {
    if (!props.isOpen || !subscription) return
    setTitleOverride(subscription.titleOverride ?? "")
    setCategoryId(subscription.categoryId ?? uncategorizedValue)
  }, [props.isOpen, subscription])

  const close = () => {
    props.onClearError()
    props.onOpenChange(false)
  }

  const saveTitle = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (!subscription) return
    props.onClearError()
    setIsSaving(true)
    const saved = await props.onUpdate(subscription.subscriptionId, {
      titleOverride: titleOverride.trim() || null,
    })
    setIsSaving(false)
    if (!saved) setTitleOverride(subscription.titleOverride ?? "")
  }

  const assign = async (value: string) => {
    if (!subscription) return
    const previous = categoryId
    setCategoryId(value)
    props.onClearError()
    setIsAssigning(true)
    const saved = await props.onUpdate(subscription.subscriptionId, {
      categoryId: value === uncategorizedValue ? null : value,
    })
    setIsAssigning(false)
    if (!saved) setCategoryId(previous)
  }

  const deleteSubscription = async () => {
    if (!subscription) return
    props.onClearError()
    setIsDeleting(true)
    const deleted = await props.onDelete(subscription.subscriptionId)
    setIsDeleting(false)
    setIsDeleteOpen(false)
    if (deleted) props.onOpenChange(false)
  }

  const categoryOptions = [
    { value: uncategorizedValue, label: i18n._("reader.uncategorized") },
    ...props.categories.map((category) => ({
      value: category.categoryId,
      label: category.title,
    })),
  ]
  const linkTarget = props.linkOpenMode === "NEW_TAB" ? "_blank" : undefined
  const linkRel = linkTarget ? "noopener noreferrer" : undefined

  return (
    <Dialog
      isOpen={props.isOpen && Boolean(subscription)}
      aria-label={i18n._("reader.editSubscription")}
      onOpenChange={(open) => {
        if (!open) close()
      }}
      purpose="form"
      width="min(620px, calc(100vw - 24px))"
      maxHeight="min(720px, calc(100dvh - 24px))"
      className="reader-subscription-edit-dialog"
    >
      <Layout
        height="fill"
        padding={0}
        header={(
          <DialogHeader
            title={i18n._("reader.editSubscription")}
            subtitle={subscription
              ? i18n._("reader.manageFeedDescription", { title: subscription.title })
              : undefined}
            hasDivider
            className="reader-dialog-header"
          />
        )}
        content={(
          <LayoutContent padding={5} className="reader-subscription-edit-content">
            {props.mutationError ? (
              <Banner
                container="section"
                status="error"
                title={i18n._("reader.feedManagementError")}
                description={props.mutationError}
              />
            ) : null}
            {subscription ? (
              <div className="reader-subscription-edit-stack">
                <dl className="reader-feed-addresses">
                  <div>
                    <dt>{i18n._("reader.feedUrl")}</dt>
                    <dd>
                      <a href={subscription.feedUrl} target={linkTarget} rel={linkRel}>
                        {subscription.feedUrl}
                      </a>
                    </dd>
                  </div>
                  <div>
                    <dt>{i18n._("reader.websiteUrl")}</dt>
                    <dd>
                      {subscription.siteUrl ? (
                        <a href={subscription.siteUrl} target={linkTarget} rel={linkRel}>
                          {subscription.siteUrl}
                        </a>
                      ) : i18n._("reader.feedSiteUnavailable")}
                    </dd>
                  </div>
                </dl>
                <form onSubmit={saveTitle} noValidate>
                  <FormLayout>
                    <TextInput
                      label={i18n._("reader.feedCustomTitle")}
                      description={i18n._("reader.feedCustomTitleDescription")}
                      value={titleOverride}
                      onChange={setTitleOverride}
                      placeholder={subscription.title}
                      width="100%"
                    />
                    <div className="reader-category-form-actions">
                      <Button
                        label={i18n._("reader.saveFeed")}
                        type="submit"
                        isLoading={isSaving}
                        isDisabled={
                          titleOverride.trim() === (subscription.titleOverride ?? "")
                        }
                        variant="primary"
                      />
                    </div>
                  </FormLayout>
                </form>
                <Selector
                  label={i18n._("reader.feedCategory")}
                  options={categoryOptions}
                  value={categoryId}
                  onChange={(value) => void assign(value)}
                  isLoading={isAssigning}
                  hasSearch={props.categories.length > 8}
                  placement="below"
                  width="100%"
                />
                <div className="reader-feed-danger-zone">
                  <div>
                    <div className="reader-preference-label">
                      {i18n._("reader.deleteFeed")}
                    </div>
                    <div className="reader-preference-description">
                      {i18n._("reader.deleteFeedDescription")}
                    </div>
                  </div>
                  <Button
                    label={i18n._("reader.deleteFeed")}
                    onClick={() => setIsDeleteOpen(true)}
                    variant="destructive"
                  />
                </div>
              </div>
            ) : null}
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
      <AlertDialog
        isOpen={isDeleteOpen}
        onOpenChange={(open) => {
          if (!isDeleting) setIsDeleteOpen(open)
        }}
        title={i18n._("reader.deleteFeedTitle")}
        description={i18n._("reader.deleteFeedConfirmation", {
          title: subscription?.title ?? "",
        })}
        actionLabel={i18n._("reader.deleteFeed")}
        cancelLabel={i18n._("common.cancel")}
        isActionLoading={isDeleting}
        onAction={() => void deleteSubscription()}
      />
    </Dialog>
  )
}
