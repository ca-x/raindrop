import { useCallback, useEffect, useRef, useState } from "react"

import { ApiClientError } from "../../../shared/api/client"
import { loadTamboSettings, saveTamboSettings, type TamboSettings } from "./api"

const settingsEvent = "raindrop:tambo-settings"
type SettingsError = "load" | "save" | "conflict" | null

export function useTamboSettings(userId?: string) {
  const generation = useRef(0)
  const [state, setState] = useState<{ owner: string | undefined; settings: TamboSettings | null; error: SettingsError; saving: boolean }>({
    owner: userId, settings: null, error: null, saving: false,
  })
  const [reloadVersion, setReloadVersion] = useState(0)
  const reload = useCallback(() => setReloadVersion((value) => value + 1), [])

  useEffect(() => {
    const current = ++generation.current
    setState({ owner: userId, settings: null, error: null, saving: false })
    if (!userId) return
    const request = new AbortController()
    void loadTamboSettings(request.signal).then((settings) => {
      if (generation.current === current && !request.signal.aborted) setState({ owner: userId, settings, error: null, saving: false })
    }).catch(() => {
      if (!request.signal.aborted && generation.current === current) setState({ owner: userId, settings: null, error: "load", saving: false })
    })
    const onSaved = (event: Event) => {
      const detail = (event as CustomEvent<{ userId: string; settings: TamboSettings }>).detail
      if (detail?.userId === userId && generation.current === current) {
        request.abort()
        setState({ owner: userId, settings: detail.settings, error: null, saving: false })
      }
    }
    window.addEventListener(settingsEvent, onSaved)
    return () => { request.abort(); generation.current++; window.removeEventListener(settingsEvent, onSaved) }
  }, [userId, reloadVersion])

  const visible = state.owner === userId ? state : { owner: userId, settings: null, error: null, saving: false }
  const save = async (enabled: boolean, csrfToken: string): Promise<boolean> => {
    if (!userId || !visible.settings || visible.saving) return false
    const current = generation.current
    setState((state) => ({ ...state, saving: true, error: null }))
    try {
      const settings = await saveTamboSettings(visible.settings, enabled, csrfToken)
      if (generation.current !== current) return false
      window.dispatchEvent(new CustomEvent(settingsEvent, { detail: { userId, settings } }))
      return true
    } catch (error) {
      if (generation.current === current) {
        const conflict = error instanceof ApiClientError && error.status === 409
        const settings = conflict ? await loadTamboSettings().catch(() => null) : visible.settings
        if (generation.current === current) setState({ owner: userId, settings, error: conflict ? "conflict" : "save", saving: false })
      }
      return false
    }
  }
  return { settings: visible.settings, error: visible.error, isSaving: visible.saving, reload, save }
}
