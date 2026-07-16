import { AppShell } from "@astryxdesign/core/AppShell"
import { Banner } from "@astryxdesign/core/Banner"
import { Center } from "@astryxdesign/core/Center"
import { Spinner } from "@astryxdesign/core/Spinner"
import { useLingui } from "@lingui/react"
import { useState } from "react"

import { LoginPage } from "../features/auth/LoginPage"
import type { SessionResponse } from "../features/auth/session"
import { ReadyPage } from "../features/reader/ReadyPage"
import { SetupPage } from "../features/setup/SetupPage"
import { useInitialAppState } from "./useInitialAppState"

export function App() {
  const { i18n } = useLingui()
  const state = useInitialAppState()
  const [override, setOverride] = useState<
    { phase: "login" } | { phase: "ready"; session: SessionResponse } | null
  >(null)

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
    return (
      <LoginPage
        onAuthenticated={(session) => setOverride({ phase: "ready", session })}
      />
    )
  }
  if (override?.phase === "ready") {
    return (
      <ReadyPage
        session={override.session}
        onLoggedOut={() => setOverride({ phase: "login" })}
      />
    )
  }

  switch (state.value.phase) {
    case "setup":
      return (
        <SetupPage
          mode={state.value.bootstrap.setupMode!}
          onAuthenticated={(session) => setOverride({ phase: "ready", session })}
          onLoginRequired={() => setOverride({ phase: "login" })}
        />
      )
    case "login":
      return (
        <LoginPage onAuthenticated={(session) => setOverride({ phase: "ready", session })} />
      )
    case "ready":
      return (
        <ReadyPage
          session={state.value.session}
          onLoggedOut={() => setOverride({ phase: "login" })}
        />
      )
  }
}
