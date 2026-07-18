import { useEntryAnimation, type EntryAnimationPreset } from "@astryxdesign/core/hooks"
import * as stylex from "@stylexjs/stylex"
import type { ReactNode } from "react"

interface MountTransitionProps {
  children: ReactNode
  preset?: EntryAnimationPreset
}

export function MountTransition({
  children,
  preset = "fadeIn",
}: MountTransitionProps) {
  const entryStyle = useEntryAnimation(preset)
  return <div {...stylex.props(entryStyle)}>{children}</div>
}
