import { useLingui } from "@lingui/react"
import { useEffect, useState } from "react"

import { useViewportMode } from "../../shared/responsive/useViewportMode"
import { logout } from "../auth/api"
import type { SessionResponse } from "../auth/session"
import { useAiSettingsController } from "../ai/model/useAiSettingsController"
import { usePreferencesController } from "../preferences/model/usePreferencesController"
import { useProfileController } from "../profile/model/useProfileController"
import { useTranslationSettingsController } from "../translation/model/useTranslationSettingsController"
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
  const profileController = useProfileController({
    csrfToken: session.csrfToken,
    initialProfile: {
      userId: session.user.id,
      username: session.user.username,
      displayName: null,
      email: session.user.email,
    },
    onUnauthenticated: onLoggedOut,
  })
  const aiSettingsController = useAiSettingsController({
    csrfToken: session.csrfToken,
    onUnauthenticated: onLoggedOut,
  })
  const translationController = useTranslationSettingsController({
    csrfToken: session.csrfToken,
    onUnauthenticated: onLoggedOut,
  })

  useEffect(() => {
    void controller.load()
    void preferencesController.load()
    void profileController.load()
    void aiSettingsController.load()
    void translationController.load()
    return () => {
      preferencesController.cancelLoad()
      profileController.cancel()
      aiSettingsController.cancel()
      translationController.cancel()
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
  ])

  const signOut = async () => {
    setSessionError(null)
    try {
      aiSettingsController.cancel()
      translationController.cancel()
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
    profileController,
    aiSettingsController,
    translationController,
    username: session.user.username,
    email: session.user.email,
    onLogout: signOut,
    onUnauthenticated: onLoggedOut,
    sessionError,
  }
  if (viewportMode === "compact") {
    return <ReadyMobilePage {...workspaceProps} />
  }
  return <ReaderRoutes {...workspaceProps} viewportMode={viewportMode} />
}
