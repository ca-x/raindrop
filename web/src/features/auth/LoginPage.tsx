import { AppShell } from "@astryxdesign/core/AppShell"
import { Banner } from "@astryxdesign/core/Banner"
import { Button } from "@astryxdesign/core/Button"
import { Card } from "@astryxdesign/core/Card"
import { Center } from "@astryxdesign/core/Center"
import { FormLayout } from "@astryxdesign/core/FormLayout"
import { Heading } from "@astryxdesign/core/Heading"
import { Stack } from "@astryxdesign/core/Stack"
import { Text } from "@astryxdesign/core/Text"
import { TextInput } from "@astryxdesign/core/TextInput"
import { useLingui } from "@lingui/react"
import { useState, type FormEvent } from "react"

import { BrandMark } from "../../shared/brand/BrandMark"
import { LocaleSwitch } from "../../shared/i18n/LocaleSwitch"
import { login } from "./api"
import type { SessionResponse } from "./session"

interface LoginPageProps {
  onAuthenticated: (session: SessionResponse) => void
}

export function LoginPage({ onAuthenticated }: LoginPageProps) {
  const { i18n } = useLingui()
  const [identifier, setIdentifier] = useState("")
  const [password, setPassword] = useState("")
  const [isLoading, setIsLoading] = useState(false)
  const [hasError, setHasError] = useState(false)

  const submit = async (event: FormEvent) => {
    event.preventDefault()
    if (!identifier.trim() || !password) {
      setHasError(true)
      return
    }
    setHasError(false)
    setIsLoading(true)
    try {
      onAuthenticated(await login({ login: identifier, password }))
    } catch {
      setHasError(true)
    } finally {
      setIsLoading(false)
    }
  }

  return (
    <AppShell contentPadding={0} height="fill" mobileNav={false} variant="wash">
      <div className="raindrop-auth-frame">
        <Stack gap={4} className="raindrop-auth-context">
          <Stack direction="horizontal" gap={3} align="center">
            <BrandMark size="lg" decorative />
            <Text type="supporting" color="accent">Raindrop</Text>
          </Stack>
          <Heading level={2} textWrap="balance" className="raindrop-reading-heading">
            {i18n._("login.contextTitle")}
          </Heading>
          <Text type="body" color="secondary" textWrap="pretty" as="p">
            {i18n._("login.description")}
          </Text>
        </Stack>
        <Center minHeight="100%" width="100%" className="raindrop-auth-form">
          <Card maxWidth={460} width="100%" padding={8} className="raindrop-auth-card">
            <Stack gap={5}>
              <Stack direction="horizontal" gap={3} justify="between" align="center" wrap="wrap">
                <Stack direction="horizontal" gap={3} align="center">
                  <BrandMark size="sm" />
                  <Stack gap={1}>
                    <Text type="supporting" color="accent">
                      {i18n._("login.eyebrow")}
                    </Text>
                    <Heading level={1} textWrap="balance" className="raindrop-reading-heading">
                      {i18n._("login.title")}
                    </Heading>
                  </Stack>
                </Stack>
                <LocaleSwitch />
              </Stack>
              {hasError ? (
                <Banner
                  status="error"
                  title={i18n._("login.error")}
                  description={i18n._("login.errorDescription")}
                />
              ) : null}
              <form onSubmit={submit} noValidate>
                <Stack gap={4}>
                  <FormLayout>
                    <TextInput
                      label={i18n._("login.username")}
                      value={identifier}
                      onChange={setIdentifier}
                      htmlName="login"
                      isRequired
                      width="100%"
                    />
                    <TextInput
                      label={i18n._("login.password")}
                      type="password"
                      value={password}
                      onChange={setPassword}
                      htmlName="password"
                      isRequired
                      width="100%"
                    />
                  </FormLayout>
                  <Button
                    type="submit"
                    label={i18n._("login.continue")}
                    variant="primary"
                    size="lg"
                    isLoading={isLoading}
                    className="raindrop-pressable"
                    style={{ minHeight: 44 }}
                  />
                </Stack>
              </form>
            </Stack>
          </Card>
        </Center>
      </div>
    </AppShell>
  )
}
