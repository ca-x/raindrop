import { AppShell } from "@astryxdesign/core/AppShell"
import { Banner } from "@astryxdesign/core/Banner"
import { Center } from "@astryxdesign/core/Center"
import { Spinner } from "@astryxdesign/core/Spinner"
import { useLingui } from "@lingui/react"

import { LoginPage } from "../features/auth/LoginPage"
import { ReadyPage } from "../features/reader/ReadyPage"
import { SetupPage } from "../features/setup/SetupPage"
import { useInitialAppState } from "./useInitialAppState"

export function App() {
  const { i18n } = useLingui()
  const state = useInitialAppState()

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

  switch (state.value.phase) {
    case "setup":
      return <SetupPage />
    case "login":
      return <LoginPage />
    case "ready":
      return <ReadyPage />
  }
}
