import { Navigate, useLocation, useNavigate } from "react-router-dom"
import { useCallback, useEffect, useRef, useState } from "react"

import type { ViewportMode } from "../../../shared/responsive/useViewportMode"
import type { AiSettingsController } from "../../ai/model/useAiSettingsController"
import type { PreferencesController } from "../../preferences/model/usePreferencesController"
import type { ProfileController } from "../../profile/model/useProfileController"
import type { TranslationSettingsController } from "../../translation/model/useTranslationSettingsController"
import { adjacentUnreadSource, type UnreadSourceDirection } from "../model/unreadSourceNavigation"
import { sourceKey } from "../model/types"
import type { ReaderController } from "../model/useReaderController"
import { ReaderShell } from "../layout/ReaderShell"
import {
  parseReaderPath,
  pathForEntry,
  pathForSource,
  sameReaderSource,
} from "./readerRoute"

interface ReaderRoutesProps {
  controller: ReaderController
  preferencesController: PreferencesController
  profileController?: ProfileController
  aiSettingsController?: AiSettingsController
  translationController?: TranslationSettingsController
  username: string
  onLogout: () => Promise<void>
  onUnauthenticated?: () => void
  sessionError?: string | null
  viewportMode: ViewportMode
}

export function ReaderRoutes(props: ReaderRoutesProps) {
  const location = useLocation()
  const navigate = useNavigate()
  const readerStateRef = useRef(props.controller.state)
  readerStateRef.current = props.controller.state
  const navigateUnreadSource = useCallback(async (direction: UnreadSourceDirection) => {
    const source = adjacentUnreadSource(readerStateRef.current, direction)
    if (source) await navigate(pathForSource(source))
  }, [navigate])
  const route = parseReaderPath(location.pathname)
  const [cursorEntryId, setCursorEntryId] = useState<string | null>(route?.entryId ?? null)
  const [cursorFocusNonce, setCursorFocusNonce] = useState(0)
  const previousRoute = useRef({ sourcePath: route?.sourcePath, entryId: route?.entryId })
  const isSourceReady = Boolean(
    route &&
    sameReaderSource(route.source, props.controller.state.selectedSource) &&
    props.controller.state.paneStatus.queue === "ready",
  )
  const queue = isSourceReady
    ? props.controller.state.queueBySourceKey[sourceKey(props.controller.state.selectedSource)] ?? []
    : []

  useEffect(() => {
    if (!route || sameReaderSource(route.source, props.controller.state.selectedSource)) return
    void props.controller.selectSource(route.source)
  }, [props.controller, route?.sourcePath])

  useEffect(() => {
    if (!route) return
    void props.controller.selectEntry(route.entryId)
  }, [props.controller.selectEntry, route?.entryId, route?.sourcePath])

  useEffect(() => {
    if (!route) return
    const previous = previousRoute.current
    if (!isSourceReady) {
      if (previous.sourcePath !== route.sourcePath) setCursorEntryId(null)
      previousRoute.current = { sourcePath: route.sourcePath, entryId: route.entryId }
      return
    }
    if (previous.sourcePath !== route.sourcePath) {
      setCursorEntryId(route.entryId && queue.includes(route.entryId) ? route.entryId : null)
    } else if (previous.entryId !== route.entryId && route.entryId && queue.includes(route.entryId)) {
      setCursorEntryId(route.entryId)
    } else if (!cursorEntryId && route.entryId && queue.includes(route.entryId)) {
      setCursorEntryId(route.entryId)
    } else if (previous.entryId && !route.entryId && cursorEntryId) {
      setCursorFocusNonce((value) => value + 1)
    }
    previousRoute.current = { sourcePath: route.sourcePath, entryId: route.entryId }
  }, [cursorEntryId, isSourceReady, queue, route?.entryId, route?.sourcePath])

  useEffect(() => {
    if (isSourceReady && cursorEntryId && !queue.includes(cursorEntryId)) setCursorEntryId(null)
  }, [cursorEntryId, isSourceReady, queue])

  if (!route) return <Navigate to="/reader/unread" replace />
  const readerQueuePath = (location.state as { readerQueuePath?: unknown } | null)?.readerQueuePath
  const markOpenedEntryRead = (entryId: string) => {
    const entry = props.controller.state.entriesById[entryId]
      ?? props.controller.state.detailsById[entryId]
    if (entry && !entry.isRead) void props.controller.toggleRead(entryId)
  }

  return (
    <ReaderShell
      {...props}
      route={route}
      isSourceReady={isSourceReady}
      cursorEntryId={cursorEntryId}
      cursorFocusNonce={cursorFocusNonce}
      onCursorChange={(entryId) => {
        setCursorEntryId(entryId)
        setCursorFocusNonce((value) => value + 1)
      }}
      onSelectSource={(source) => navigate(pathForSource(source))}
      onSelectEntry={(entryId) => {
        setCursorEntryId(entryId)
        markOpenedEntryRead(entryId)
        const path = pathForEntry(route.sourcePath, entryId)
        if (route.entryId) {
          navigate(path, {
            replace: true,
            state: readerQueuePath === route.sourcePath
              ? { readerQueuePath: route.sourcePath }
              : null,
          })
          return
        }
        navigate(path, { state: { readerQueuePath: route.sourcePath } })
      }}
      onOpenEntryFromHotkey={(entryId) => {
        const path = pathForEntry(route.sourcePath, entryId)
        const hasQueueOrigin = !route.entryId || readerQueuePath === route.sourcePath
        navigate(path, {
          replace: Boolean(route.entryId),
          state: hasQueueOrigin ? { readerQueuePath: route.sourcePath } : null,
        })
      }}
      onNextUnreadSource={() => navigateUnreadSource(1)}
      onPreviousUnreadSource={() => navigateUnreadSource(-1)}
      onBack={() => {
        if (readerQueuePath === route.sourcePath) navigate(-1)
        else navigate(route.sourcePath, { replace: true })
      }}
    />
  )
}
