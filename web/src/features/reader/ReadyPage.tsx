import { useLingui } from "@lingui/react"
import { useCallback, useEffect, useRef, useState } from "react"
import { useLocation } from "react-router"

import { useViewportMode } from "../../shared/responsive/useViewportMode"
import { logout } from "../auth/api"
import type { SessionResponse } from "../auth/session"
import { useAiSettingsController } from "../ai/model/useAiSettingsController"
import { useBackupController } from "../backups/model/useBackupController"
import { usePreferencesController } from "../preferences/model/usePreferencesController"
import { useProfileController } from "../profile/model/useProfileController"
import { useTranslationSettingsController } from "../translation/model/useTranslationSettingsController"
import { useReaderController } from "./model/useReaderController"
import type { ReaderCache } from "./cache/readerCache"
import { ReadyMobilePage } from "./ReadyMobilePage"
import { ReaderRoutes } from "./routes/ReaderRoutes"
import { parseReaderPath } from "./routes/readerRoute"

interface ReadyPageProps {
  session: SessionResponse
  onLoggedOut: () => void
  readerCache?: ReaderCache
}

export function ReadyPage({ session, onLoggedOut, readerCache }: ReadyPageProps) {
  const { i18n } = useLingui()
  const location = useLocation()
  const viewportMode = useViewportMode()
  const [sessionError, setSessionError] = useState<string | null>(null)
  const clearReaderCacheRef = useRef<() => Promise<void>>(async () => undefined)
  const sessionExpiredRef = useRef(false)
  const handleUnauthenticated = useCallback(async () => {
    if (sessionExpiredRef.current) return
    sessionExpiredRef.current = true
    await clearReaderCacheRef.current()
    onLoggedOut()
  }, [onLoggedOut])
  const controller = useReaderController({
    csrfToken: session.csrfToken,
    userId: session.user.id,
    initialSource: parseReaderPath(location.pathname)?.source,
    cache: readerCache,
    onUnauthenticated: handleUnauthenticated,
  })
  clearReaderCacheRef.current = controller.clearCache
  const preferencesController = usePreferencesController({
    csrfToken: session.csrfToken,
    onUnauthenticated: handleUnauthenticated,
  })
  const profileController = useProfileController({
    csrfToken: session.csrfToken,
    initialProfile: {
      userId: session.user.id,
      username: session.user.username,
      displayName: null,
      email: session.user.email,
    },
    onUnauthenticated: handleUnauthenticated,
  })
  const aiSettingsController = useAiSettingsController({
    csrfToken: session.csrfToken,
    onUnauthenticated: handleUnauthenticated,
  })
  const translationController = useTranslationSettingsController({
    csrfToken: session.csrfToken,
    onUnauthenticated: handleUnauthenticated,
  })
  const backupController = useBackupController({
    csrfToken: session.csrfToken,
    onUnauthenticated: handleUnauthenticated,
  })

  useEffect(() => {
    void controller.load()
    void preferencesController.load()
    void profileController.load()
    void aiSettingsController.load()
    void translationController.load()
    void backupController.load()
    return () => {
      preferencesController.cancelLoad()
      profileController.cancel()
      aiSettingsController.cancel()
      translationController.cancel()
      backupController.cancel()
    }
  }, [
    aiSettingsController.cancel,
    aiSettingsController.load,
    controller.load,
    preferencesController.cancelLoad,
    preferencesController.load,
    profileController.cancel,
    profileController.load,
    translationController.cancel,
    translationController.load,
    backupController.cancel,
    backupController.load,
  ])

  const signOut = async () => {
    setSessionError(null)
    try {
      aiSettingsController.cancel()
      translationController.cancel()
      backupController.cancel()
      await logout(session.csrfToken)
      await controller.clearCache()
      preferencesController.clearHint()
      onLoggedOut()
    } catch {
      setSessionError(i18n._("reader.logoutError"))
    }
  }

  const workspaceProps = {
    controller,
    preferencesController,
    profileController,
    aiSettingsController,
    translationController,
    backupController,
    username: session.user.username,
    email: session.user.email,
    onLogout: signOut,
    onUnauthenticated: handleUnauthenticated,
    sessionError,
  }
  if (viewportMode === "compact") {
    return <ReadyMobilePage {...workspaceProps} />
  }
  return <ReaderRoutes {...workspaceProps} viewportMode={viewportMode} />
}
