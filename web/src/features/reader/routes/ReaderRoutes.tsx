import { Navigate, useLocation, useNavigate } from "react-router-dom"
import { useEffect } from "react"

import type { ViewportMode } from "../../../shared/responsive/useViewportMode"
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

  useEffect(() => {
    if (!route || sameReaderSource(route.source, props.controller.state.selectedSource)) return
    void props.controller.selectSource(route.source)
  }, [props.controller, route?.sourcePath])

  useEffect(() => {
    if (!route) return
    void props.controller.selectEntry(route.entryId)
  }, [props.controller.selectEntry, route?.entryId, route?.sourcePath])

  if (!route) return <Navigate to="/reader/unread" replace />
  const readerQueuePath = (location.state as { readerQueuePath?: unknown } | null)?.readerQueuePath

  return (
    <ReaderShell
      {...props}
      route={route}
      onSelectSource={(source) => navigate(pathForSource(source))}
      onSelectEntry={(entryId) => {
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
      onBack={() => {
        if (readerQueuePath === route.sourcePath) navigate(-1)
        else navigate(route.sourcePath, { replace: true })
      }}
    />
  )
}
