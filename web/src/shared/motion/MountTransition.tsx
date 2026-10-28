import type { EntryAnimationPreset } from "@astryxdesign/core/hooks"
import type { ReactNode } from "react"

interface MountTransitionProps {
  children: ReactNode
  preset?: EntryAnimationPreset
}

export function MountTransition({
  children,
  preset = "fadeIn",
}: MountTransitionProps) {
  return <div className="raindrop-mount-transition" data-preset={preset}>{children}</div>
}
