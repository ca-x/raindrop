import { useEffect, useRef, useState } from "react"

import {
  clampReadingFontScale,
  DEFAULT_READING_FONT_SCALE,
  READING_FONT_SCALE_STEP,
} from "../../preferences/model/preferenceTypes"
import type { PreferencesController } from "../../preferences/model/usePreferencesController"

// Keep repeated key presses visible while serializing preference writes.
export function useReadingFontScale(controller: Pick<PreferencesController, "preferences" | "isSaving" | "save">) {
  const [target, setTarget] = useState<{ scale: number } | null>(null)
  const targetRef = useRef(target)
  const isWriting = useRef(false)

  useEffect(() => {
    if (!target || controller.isSaving || isWriting.current) return
    const { scale } = target
    isWriting.current = true
    void controller.save({ ...controller.preferences, readingFontScale: scale }).then((saved) => {
      isWriting.current = false
      if (!saved || targetRef.current?.scale === scale) {
        targetRef.current = null
        setTarget(null)
      } else if (targetRef.current) {
        setTarget({ ...targetRef.current })
      }
    })
  }, [controller.isSaving, controller.preferences, controller.save, target])

  return {
    scale: target?.scale ?? controller.preferences.readingFontScale,
    isPending: target !== null,
    change: (direction: 1 | -1 | 0) => {
      const current = targetRef.current?.scale ?? controller.preferences.readingFontScale
      const scale = direction === 0
        ? DEFAULT_READING_FONT_SCALE
        : clampReadingFontScale(current + direction * READING_FONT_SCALE_STEP)
      if (scale === current) return
      targetRef.current = { scale }
      setTarget(targetRef.current)
    },
  }
}
