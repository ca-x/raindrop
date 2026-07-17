import { Button } from "@astryxdesign/core/Button"
import { Dialog, DialogHeader } from "@astryxdesign/core/Dialog"
import { FormLayout } from "@astryxdesign/core/FormLayout"
import { Layout, LayoutContent, LayoutFooter } from "@astryxdesign/core/Layout"
import { TextInput } from "@astryxdesign/core/TextInput"
import { useLingui } from "@lingui/react"
import { useEffect, useState, type FormEvent } from "react"

import { isCreateSubscriptionRequest } from "../api/subscription.generated"

interface SubscriptionDialogProps {
  isOpen: boolean
  mutationError: string | null
  onOpenChange: (isOpen: boolean) => void
  onClearError: () => void
  onAdd: (url: string) => Promise<void>
}

export function SubscriptionDialog(props: SubscriptionDialogProps) {
  const { i18n } = useLingui()
  const [url, setUrl] = useState("")
  const [validationError, setValidationError] = useState<string | null>(null)
  const [isSubmitting, setIsSubmitting] = useState(false)
  const [isSettled, setIsSettled] = useState(false)

  useEffect(() => {
    if (!isSettled) return
    setIsSettled(false)
    setIsSubmitting(false)
    if (props.mutationError) return
    setUrl("")
    props.onOpenChange(false)
  }, [isSettled, props.mutationError, props.onOpenChange])

  const close = () => {
    if (isSubmitting) return
    setValidationError(null)
    props.onClearError()
    props.onOpenChange(false)
  }

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    const value = url.trim()
    if (!isCreateSubscriptionRequest({ url: value })) {
      setValidationError(i18n._("reader.feedUrlInvalid"))
      return
    }
    setValidationError(null)
    props.onClearError()
    setIsSubmitting(true)
    try {
      await props.onAdd(value)
    } finally {
      setIsSettled(true)
    }
  }

  const error = validationError ?? props.mutationError
  return (
    <Dialog
      isOpen={props.isOpen}
      onOpenChange={(open) => {
        if (!open) close()
      }}
      purpose="form"
      width={520}
      className="reader-subscription-dialog"
    >
      <Layout
        height="auto"
        padding={0}
        header={
          <DialogHeader
            title={i18n._("reader.addSubscription")}
            subtitle={i18n._("reader.addSubscriptionDescription")}
            onOpenChange={(open) => {
              if (!open) close()
            }}
            hasDivider
          />
        }
        content={
          <LayoutContent padding={4} isScrollable={false}>
            <form id="reader-add-subscription" onSubmit={submit} noValidate>
              <FormLayout>
                <TextInput
                  label={i18n._("reader.feedUrl")}
                  type="text"
                  value={url}
                  onChange={(value) => {
                    setUrl(value)
                    setValidationError(null)
                    if (props.mutationError) props.onClearError()
                  }}
                  placeholder="https://example.com/feed.xml"
                  status={error ? { type: "error", message: error } : undefined}
                  isRequired
                  width="100%"
                  className="reader-touch-target"
                />
              </FormLayout>
            </form>
          </LayoutContent>
        }
        footer={
          <LayoutFooter hasDivider padding={3}>
            <div className="reader-dialog-actions">
              <Button label={i18n._("common.cancel")} onClick={close} variant="secondary" />
              <Button
                label={i18n._("reader.addSubscription")}
                type="submit"
                form="reader-add-subscription"
                isLoading={isSubmitting}
                variant="primary"
              />
            </div>
          </LayoutFooter>
        }
      />
    </Dialog>
  )
}
