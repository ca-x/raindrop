import { useLingui } from "@lingui/react"
import { useEffect, useState } from "react"

import { useViewportMode } from "../../shared/responsive/useViewportMode"
import { logout } from "../auth/api"
import type { SessionResponse } from "../auth/session"
import { useAiSettingsController } from "../ai/model/useAiSettingsController"
import { usePreferencesController } from "../preferences/model/usePreferencesController"
import { useReaderController } from "./model/useReaderController"
import { ReadyMobilePage } from "./ReadyMobilePage"
import { ReaderRoutes } from "./routes/ReaderRoutes"

interface ReadyPageProps {
  session: SessionResponse
  onLoggedOut: () => void
}

export function ReadyPage({ session, onLoggedOut }: ReadyPageProps) {
  const { i18n } = useLingui()
  const viewportMode = useViewportMode()
  const [sessionError, setSessionError] = useState<string | null>(null)
  const controller = useReaderController({
    csrfToken: session.csrfToken,
    onUnauthenticated: onLoggedOut,
  })
  const preferencesController = usePreferencesController({
    csrfToken: session.csrfToken,
    onUnauthenticated: onLoggedOut,
  })
  const aiSettingsController = useAiSettingsController({
    csrfToken: session.csrfToken,
    onUnauthenticated: onLoggedOut,
  })

  useEffect(() => {
    void controller.load()
    void preferencesController.load()
    void aiSettingsController.load()
    return () => {
      preferencesController.cancelLoad()
      aiSettingsController.cancel()
    }
  }, [
    aiSettingsController.cancel,
    aiSettingsController.load,
    controller.load,
    preferencesController.cancelLoad,
    preferencesController.load,
  ])

  const signOut = async () => {
    setSessionError(null)
    try {
      aiSettingsController.cancel()
      await logout(session.csrfToken)
      preferencesController.clearHint()
      onLoggedOut()
    } catch {
      setSessionError(i18n._("reader.logoutError"))
    }
  }

  const workspaceProps = {
    controller,
    preferencesController,
    aiSettingsController,
    username: session.user.username,
    onLogout: signOut,
    onUnauthenticated: onLoggedOut,
    sessionError,
  }
  if (viewportMode === "compact") {
    return <ReadyMobilePage {...workspaceProps} />
  }
  return <ReaderRoutes {...workspaceProps} viewportMode={viewportMode} />
}
