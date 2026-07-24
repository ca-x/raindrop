import { AppShell } from "@astryxdesign/core/AppShell"
import { Banner } from "@astryxdesign/core/Banner"
import { Center } from "@astryxdesign/core/Center"
import { Spinner } from "@astryxdesign/core/Spinner"
import { useLingui } from "@lingui/react"
import { useCallback, useState } from "react"
import { Navigate, useLocation } from "react-router-dom"

import { LoginPage } from "../features/auth/LoginPage"
import type { SessionResponse } from "../features/auth/session"
import { ReadyPage } from "../features/reader/ReadyPage"
import { DEFAULT_READER_PATH } from "../features/reader/routes/readerRoute"
import { SetupPage } from "../features/setup/SetupPage"
import { useInitialAppState } from "./useInitialAppState"

export function App() {
  const { i18n } = useLingui()
  const location = useLocation()
  const state = useInitialAppState()
  const [override, setOverride] = useState<
    | { phase: "login" }
    | { phase: "ready"; session: SessionResponse; destination: string }
    | null
  >(null)
  const showLogin = useCallback(() => setOverride({ phase: "login" }), [])
  const showReady = useCallback(
    (session: SessionResponse) => {
      setOverride({
        phase: "ready",
        session,
        destination: readerReturnPath(location.state),
      })
    },
    [location.state],
  )

  if (state.status === "loading") {
    return (
      <AppShell contentPadding={0} height="fill" mobileNav={false}>
        <Center minHeight="100%">
          <Spinner label={i18n._("app.loading")} />
        </Center>
      </AppShell>
    )
  }

  if (state.status === "error") {
    return (
      <AppShell contentPadding={4} height="fill" mobileNav={false}>
        <Banner
          status="error"
          title={i18n._("app.loadError")}
          description={i18n._("app.loadErrorDescription")}
        />
      </AppShell>
    )
  }

  if (override?.phase === "login") {
    if (location.pathname !== "/login") {
      return <Navigate to="/login" replace state={readerReturnState(location.pathname)} />
    }
    return (
      <LoginPage
        onAuthenticated={showReady}
      />
    )
  }
  if (override?.phase === "ready") {
    if (location.pathname === "/login" || location.pathname === "/") {
      return <Navigate to={override.destination} replace />
    }
    return (
      <ReadyPage
        session={override.session}
        onLoggedOut={showLogin}
      />
    )
  }

  switch (state.value.phase) {
    case "setup":
      if (location.pathname !== "/") return <Navigate to="/" replace />
      return (
        <SetupPage
          mode={state.value.bootstrap.setupMode!}
          onAuthenticated={showReady}
          onLoginRequired={showLogin}
        />
      )
    case "login":
      if (location.pathname !== "/login") {
        return <Navigate to="/login" replace state={readerReturnState(location.pathname)} />
      }
      return <LoginPage onAuthenticated={showReady} />
    case "ready":
      if (location.pathname === "/login" || location.pathname === "/") {
        return <Navigate to={DEFAULT_READER_PATH} replace />
      }
      return (
        <ReadyPage
          session={state.value.session}
          onLoggedOut={showLogin}
        />
      )
  }
}

function readerReturnState(pathname: string): { returnTo: string } | null {
  return pathname.startsWith("/reader/") ? { returnTo: pathname } : null
}

function readerReturnPath(state: unknown): string {
  if (
    typeof state === "object" &&
    state !== null &&
    "returnTo" in state &&
    typeof state.returnTo === "string" &&
    state.returnTo.startsWith("/reader/")
  ) {
    return state.returnTo
  }
  return DEFAULT_READER_PATH
}
