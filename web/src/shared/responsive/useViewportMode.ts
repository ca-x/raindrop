import { useEffect, useState } from "react"

export type ViewportMode = "compact" | "medium" | "wide"

const compactQuery = "(max-width: 719px)"
const wideQuery = "(min-width: 1100px)"

export function useViewportMode(): ViewportMode {
  const [mode, setMode] = useState<ViewportMode>(() => currentMode())

  useEffect(() => {
    const compactMedia = window.matchMedia(compactQuery)
    const wideMedia = window.matchMedia(wideQuery)
    const update = () => setMode(modeFromMedia(compactMedia, wideMedia))
    update()
    compactMedia.addEventListener("change", update)
    wideMedia.addEventListener("change", update)
    return () => {
      compactMedia.removeEventListener("change", update)
      wideMedia.removeEventListener("change", update)
    }
  }, [])

  return mode
}

function currentMode(): ViewportMode {
  return modeFromMedia(window.matchMedia(compactQuery), window.matchMedia(wideQuery))
}

function modeFromMedia(compact: MediaQueryList, wide: MediaQueryList): ViewportMode {
  if (compact.matches) return "compact"
  return wide.matches ? "wide" : "medium"
}
