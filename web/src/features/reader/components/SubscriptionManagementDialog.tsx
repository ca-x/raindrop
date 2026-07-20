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

interface SubscriptionManagementDialogProps {
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

export function SubscriptionManagementDialog(
  props: SubscriptionManagementDialogProps,
) {
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
    if (isSaving || isAssigning || isDeleting) return
    props.onClearError()
    props.onOpenChange(false)
  }

  const saveTitle = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (!subscription) return
    const nextTitle = titleOverride.trim() || null
    props.onClearError()
    setIsSaving(true)
    const saved = await props.onUpdate(subscription.subscriptionId, {
      titleOverride: nextTitle,
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
  const currentTitle = subscription?.title ?? i18n._("reader.feed")
  const siteUrl = subscription?.siteUrl ?? null

  return (
    <>
      <Dialog
        isOpen={props.isOpen && !isDeleteOpen}
        aria-label={i18n._("reader.manageFeed")}
        onOpenChange={(open) => {
          if (!open) close()
        }}
        purpose="form"
        width="min(560px, calc(100vw - 24px))"
        maxHeight="min(680px, calc(100dvh - 24px))"
        className="reader-subscription-management-dialog"
      >
        <Layout
          height="auto"
          padding={0}
          header={
            <DialogHeader
              title={i18n._("reader.manageFeed")}
              subtitle={i18n._("reader.manageFeedDescription", {
                title: currentTitle,
              })}
              hasDivider
              className="reader-dialog-header"
            />
          }
          content={
            <LayoutContent padding={4} isScrollable>
              <div className="reader-feed-management-content">
                {props.mutationError ? (
                  <Banner
                    container="section"
                    status="error"
                    title={i18n._("reader.feedManagementError")}
                    description={props.mutationError}
                  />
                ) : null}

                <section className="reader-feed-management-summary">
                  <div>
                    <div className="reader-preference-label">{currentTitle}</div>
                    <div className="reader-preference-description">
                      {siteUrl ?? i18n._("reader.feedSiteUnavailable")}
                    </div>
                  </div>
                  {siteUrl ? (
                    <Button
                      label={i18n._("reader.openFeedSite")}
                      href={siteUrl}
                      target={props.linkOpenMode === "NEW_TAB" ? "_blank" : undefined}
                      rel={
                        props.linkOpenMode === "NEW_TAB"
                          ? "noopener noreferrer"
                          : undefined
                      }
                      variant="secondary"
                    />
                  ) : null}
                </section>

                <form onSubmit={saveTitle} noValidate>
                  <FormLayout>
                    <TextInput
                      label={i18n._("reader.feedCustomTitle")}
                      description={i18n._("reader.feedCustomTitleDescription")}
                      value={titleOverride}
                      onChange={(value) => {
                        setTitleOverride(value)
                        if (props.mutationError) props.onClearError()
                      }}
                      placeholder={currentTitle}
                      width="100%"
                    />
                    <div className="reader-category-form-actions">
                      <Button
                        label={i18n._("reader.saveFeed")}
                        type="submit"
                        isLoading={isSaving}
                        isDisabled={
                          !subscription ||
                          titleOverride.trim() === (subscription.titleOverride ?? "")
                        }
                        variant="primary"
                      />
                    </div>
                  </FormLayout>
                </form>

                <Selector
                  label={i18n._("reader.feedCategory")}
                  description={i18n._("reader.feedCategoryDescription", {
                    title: currentTitle,
                  })}
                  options={categoryOptions}
                  value={categoryId}
                  onChange={(value) => void assign(value)}
                  isLoading={isAssigning}
                  isDisabled={!subscription}
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
                    isDisabled={!subscription}
                    variant="destructive"
                  />
                </div>
              </div>
            </LayoutContent>
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
        title={i18n._("reader.deleteFeedTitle")}
        description={i18n._("reader.deleteFeedConfirmation", {
          title: currentTitle,
        })}
        actionLabel={i18n._("reader.deleteFeed")}
        cancelLabel={i18n._("common.cancel")}
        isActionLoading={isDeleting}
        onAction={() => void deleteSubscription()}
        width="min(440px, calc(100vw - 24px))"
      />
    </>
  )
}
