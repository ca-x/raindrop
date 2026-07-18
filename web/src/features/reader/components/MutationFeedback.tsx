import { useToast } from "@astryxdesign/core/Toast"
import { useEffect } from "react"

interface MutationFeedbackProps {
  error: string | null
  isDialogOpen: boolean
  onClear: () => void
}

export function MutationFeedback({
  error,
  isDialogOpen,
  onClear,
}: MutationFeedbackProps) {
  const showToast = useToast()

  useEffect(() => {
    if (!error || isDialogOpen) return
    showToast({
      body: error,
      type: "error",
      isAutoHide: false,
      uniqueID: "reader-mutation-error",
      collisionBehavior: "overwrite",
    })
    onClear()
  }, [error, isDialogOpen, onClear, showToast])

  return null
}
