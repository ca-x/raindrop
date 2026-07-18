import { AlertDialog } from "@astryxdesign/core/AlertDialog"
import { useLingui } from "@lingui/react"

interface MarkReadDialogProps {
  isOpen: boolean
  sourceLabel: string
  isLoading: boolean
  onOpenChange: (open: boolean) => void
  onConfirm: () => Promise<boolean>
}

export function MarkReadDialog(props: MarkReadDialogProps) {
  const { i18n } = useLingui()
  return (
    <AlertDialog
      isOpen={props.isOpen}
      onOpenChange={props.onOpenChange}
      title={i18n._("reader.markAllReadTitle", { source: props.sourceLabel })}
      description={i18n._("reader.markAllReadDescription")}
      cancelLabel={i18n._("common.cancel")}
      actionLabel={i18n._("reader.markAllReadAction")}
      actionVariant="primary"
      isActionLoading={props.isLoading}
      onAction={async () => {
        if (await props.onConfirm()) props.onOpenChange(false)
      }}
    />
  )
}
