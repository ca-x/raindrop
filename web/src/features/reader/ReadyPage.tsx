import { AppShell } from "@astryxdesign/core/AppShell"
import { Banner } from "@astryxdesign/core/Banner"
import { Button } from "@astryxdesign/core/Button"
import { Center } from "@astryxdesign/core/Center"
import { EmptyState } from "@astryxdesign/core/EmptyState"
import { Layout } from "@astryxdesign/core/Layout"
import { Section } from "@astryxdesign/core/Section"
import { Stack } from "@astryxdesign/core/Stack"
import { Text } from "@astryxdesign/core/Text"
import { useLingui } from "@lingui/react"
import { useState } from "react"

import { BrandMark } from "../../shared/brand/BrandMark"
import { logout } from "../auth/api"
import type { SessionResponse } from "../auth/session"
import { useViewportMode } from "../../shared/responsive/useViewportMode"
import { ReadyMobilePage } from "./ReadyMobilePage"

interface ReadyPageProps {
  session: SessionResponse
  onLoggedOut: () => void
}

export function ReadyPage({ session, onLoggedOut }: ReadyPageProps) {
  const { i18n } = useLingui()
  const mode = useViewportMode()
  const [isLoading, setIsLoading] = useState(false)
  const [hasError, setHasError] = useState(false)

  const signOut = async () => {
    setHasError(false)
    setIsLoading(true)
    try {
      await logout(session.csrfToken)
      onLoggedOut()
    } catch {
      setHasError(true)
    } finally {
      setIsLoading(false)
    }
  }

  if (mode === "compact") {
    return (
      <ReadyMobilePage
        username={session.user.username}
        isLoading={isLoading}
        hasError={hasError}
        onLogout={signOut}
      />
    )
  }

  return (
    <AppShell contentPadding={0} height="fill" variant="section">
      {hasError ? (
        <Banner status="error" title={i18n._("login.error")} />
      ) : null}
      <Layout
        height="fill"
        padding={4}
        contentWidth={760}
        header={
          <Section variant="section" padding={3} dividers={["bottom"]}>
            <Stack direction="horizontal" gap={2} align="center">
              <BrandMark size="sm" />
              <Text type="label">Raindrop</Text>
            </Stack>
          </Section>
        }
        content={
          <Center minHeight="100%">
            <EmptyState
              headingLevel={1}
              title={i18n._("ready.title")}
              description={i18n._("ready.description")}
              actions={
                <Button
                  label={i18n._("common.logout")}
                  variant="secondary"
                  isLoading={isLoading}
                  clickAction={signOut}
                  className="raindrop-pressable"
                  style={{ minHeight: 44 }}
                />
              }
            />
          </Center>
        }
      />
    </AppShell>
  )
}
