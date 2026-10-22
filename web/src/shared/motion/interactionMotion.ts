import type { KeyboardEvent, PointerEvent } from "react"

export const interactionMotion = {
  onPointerDownCapture: (event: PointerEvent<HTMLElement>) => {
    event.currentTarget.dataset.input = "pointer"
  },
  onKeyDownCapture: (event: KeyboardEvent<HTMLElement>) => {
    event.currentTarget.dataset.input = "keyboard"
  },
}
