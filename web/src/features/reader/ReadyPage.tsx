import { useLingui } from "@lingui/react"
import { useEffect, useState } from "react"

import { useViewportMode } from "../../shared/responsive/useViewportMode"
import { logout } from "../auth/api"
import type { SessionResponse } from "../auth/session"
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

  useEffect(() => {
    void controller.load()
  }, [controller.load])

  const signOut = async () => {
    setSessionError(null)
    try {
      await logout(session.csrfToken)
      onLoggedOut()
    } catch {
      setSessionError(i18n._("reader.logoutError"))
    }
  }

  const workspaceProps = {
    controller,
    username: session.user.username,
    onLogout: signOut,
    sessionError,
  }
  if (viewportMode === "compact") {
    return <ReadyMobilePage {...workspaceProps} />
  }
  return <ReaderRoutes {...workspaceProps} viewportMode={viewportMode} />
}
