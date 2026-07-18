import { useEffect, useState } from "react"

import { loadInitialAppState, type InitialAppState } from "./bootstrap"

export type InitialAppStateResult =
  | { status: "loading" }
  | { status: "error" }
  | { status: "loaded"; value: InitialAppState }

export function useInitialAppState(): InitialAppStateResult {
  const [state, setState] = useState<InitialAppStateResult>({ status: "loading" })

  useEffect(() => {
    const controller = new AbortController()
    loadInitialAppState(controller.signal).then(
      (value) => setState({ status: "loaded", value }),
      (error: unknown) => {
        if (!controller.signal.aborted) {
          setState({ status: "error" })
        }
      },
    )
    return () => controller.abort()
  }, [])

  return state
}
