import { useEffect, useState } from "react"

export type ViewportMode = "compact" | "wide"

const compactQuery = "(max-width: 719px)"

export function useViewportMode(): ViewportMode {
  const [mode, setMode] = useState<ViewportMode>(() => currentMode())

  useEffect(() => {
    const media = window.matchMedia(compactQuery)
    const update = () => setMode(media.matches ? "compact" : "wide")
    update()
    media.addEventListener("change", update)
    return () => media.removeEventListener("change", update)
  }, [])

  return mode
}

function currentMode(): ViewportMode {
  return window.matchMedia(compactQuery).matches ? "compact" : "wide"
}
