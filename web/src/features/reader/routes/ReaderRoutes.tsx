import { Navigate, useLocation, useNavigate } from "react-router-dom"
import { useEffect, useRef, useState } from "react"

import type { ViewportMode } from "../../../shared/responsive/useViewportMode"
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
  username: string
  onLogout: () => Promise<void>
  sessionError?: string | null
  viewportMode: ViewportMode
}

export function ReaderRoutes(props: ReaderRoutesProps) {
  const location = useLocation()
  const navigate = useNavigate()
  const route = parseReaderPath(location.pathname)
  const [cursorEntryId, setCursorEntryId] = useState<string | null>(route?.entryId ?? null)
  const [cursorFocusNonce, setCursorFocusNonce] = useState(0)
  const previousRoute = useRef({ sourcePath: route?.sourcePath, entryId: route?.entryId })
  const queue = props.controller.state.queueBySourceKey[sourceKey(props.controller.state.selectedSource)] ?? []

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
  }, [cursorEntryId, queue, route?.entryId, route?.sourcePath])

  useEffect(() => {
    if (cursorEntryId && !queue.includes(cursorEntryId)) setCursorEntryId(null)
  }, [cursorEntryId, queue])

  if (!route) return <Navigate to="/reader/unread" replace />
  const readerQueuePath = (location.state as { readerQueuePath?: unknown } | null)?.readerQueuePath

  return (
    <ReaderShell
      {...props}
      route={route}
      cursorEntryId={cursorEntryId}
      cursorFocusNonce={cursorFocusNonce}
      onCursorChange={(entryId) => {
        setCursorEntryId(entryId)
        setCursorFocusNonce((value) => value + 1)
      }}
      onSelectSource={(source) => navigate(pathForSource(source))}
      onSelectEntry={(entryId) => {
        setCursorEntryId(entryId)
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
      onBack={() => {
        if (readerQueuePath === route.sourcePath) navigate(-1)
        else navigate(route.sourcePath, { replace: true })
      }}
    />
  )
}
