import { AppShell } from "@astryxdesign/core/AppShell"
import { Banner } from "@astryxdesign/core/Banner"
import { Card } from "@astryxdesign/core/Card"
import { Center } from "@astryxdesign/core/Center"
import { Heading } from "@astryxdesign/core/Heading"
import { ProgressBar } from "@astryxdesign/core/ProgressBar"
import { Section } from "@astryxdesign/core/Section"
import { Stack } from "@astryxdesign/core/Stack"
import { Text } from "@astryxdesign/core/Text"
import { useLingui } from "@lingui/react"
import { useState } from "react"

import { ApiClientError } from "../../shared/api/client"
import { BrandMark } from "../../shared/brand/BrandMark"
import { LocaleSwitch } from "../../shared/i18n/LocaleSwitch"
import { MountTransition } from "../../shared/motion/MountTransition"
import type { SessionResponse } from "../auth/session"
import { AdminStep } from "./AdminStep"
import type { SetupMode } from "../../app/bootstrap"
import { checkDatabase, completeAdminSetup, completeSetup } from "./api"
import { DatabaseStep } from "./DatabaseStep"
import { login } from "../auth/api"
import {
  initialSetupValues,
  type SetupStep,
  type SetupValues,
  validateAdmin,
  validateDatabase,
} from "./model"

interface SetupPageProps {
  mode: SetupMode
  onAuthenticated: (session: SessionResponse) => void
  onLoginRequired: () => void
}

export function SetupPage({ mode, onAuthenticated, onLoginRequired }: SetupPageProps) {
  const { i18n } = useLingui()
  const [step, setStep] = useState<SetupStep>(
    mode === "ADMIN_ONLY" ? "admin" : "database",
  )
  const [values, setValues] = useState<SetupValues>(initialSetupValues)
  const [fields, setFields] = useState<Record<string, string>>({})
  const [error, setError] = useState<"database" | "complete" | null>(null)
  const [isLoading, setIsLoading] = useState(false)

  const submitDatabase = async () => {
    if (isLoading) return
    const nextFields = validateDatabase(values)
    setFields(nextFields)
    if (Object.keys(nextFields).length) return
    setError(null)
    setIsLoading(true)
    try {
      await checkDatabase(values)
      setStep("admin")
    } catch (cause) {
      setError("database")
      mergeApiFields(cause, setFields)
    } finally {
      setIsLoading(false)
    }
  }

  const submitAdmin = async () => {
    if (isLoading) return
    const nextFields = validateAdmin(values, mode === "ADMIN_ONLY")
    setFields(nextFields)
    if (Object.keys(nextFields).length) return
    setError(null)
    setIsLoading(true)
    try {
      await (mode === "ADMIN_ONLY" ? completeAdminSetup(values) : completeSetup(values))
    } catch (cause) {
      setError("complete")
      mergeApiFields(cause, setFields)
      setIsLoading(false)
      return
    }
    try {
      onAuthenticated(await login({ login: values.username, password: values.password }))
    } catch {
      onLoginRequired()
    } finally {
      setIsLoading(false)
    }
  }

  return (
    <AppShell contentPadding={0} height="fill" mobileNav={false} variant="wash">
      <div className="raindrop-auth-frame">
        <Section variant="transparent" padding={6} className="raindrop-auth-context">
          <Stack gap={4}>
            <Stack direction="horizontal" gap={3} align="center">
              <BrandMark size="lg" decorative />
              <Text type="supporting" color="accent">Raindrop</Text>
            </Stack>
            <Heading level={2} textWrap="balance" className="raindrop-reading-heading">
              {i18n._("setup.contextTitle")}
            </Heading>
            <Text type="body" color="secondary" textWrap="pretty" as="p">
              {i18n._("setup.description")}
            </Text>
          </Stack>
        </Section>
        <Center minHeight="100%" width="100%" className="raindrop-auth-form">
          <Card maxWidth={620} width="100%" padding={8} className="raindrop-auth-card">
            <Section variant="transparent" padding={0}>
              <Stack gap={5}>
                <Stack direction="horizontal" gap={3} justify="between" align="center" wrap="wrap">
                  <Stack direction="horizontal" gap={3} align="center">
                    <BrandMark size="sm" />
                    <Stack gap={1}>
                      <Text type="supporting" color="accent">
                        {i18n._("setup.eyebrow")}
                      </Text>
                      <Heading level={1} textWrap="balance" className="raindrop-reading-heading">
                        {i18n._("setup.title")}
                      </Heading>
                    </Stack>
                  </Stack>
                  <LocaleSwitch isDisabled={isLoading} />
                </Stack>
                <ProgressBar
                  label={i18n._("setup.progress")}
                  value={mode === "ADMIN_ONLY" ? 1 : step === "database" ? 1 : 2}
                  max={mode === "ADMIN_ONLY" ? 1 : 2}
                  hasValueLabel
                  formatValueLabel={(value, max) => `${value} / ${max}`}
                />
                {error ? (
                  <Banner
                    status="error"
                    title={
                      error === "database"
                        ? i18n._("setup.databaseError")
                        : i18n._("setup.completeError")
                    }
                  />
                ) : null}
                <MountTransition key={step} preset="fadeIn">
                  {step === "database" ? (
                    <DatabaseStep
                      values={values}
                      fields={fields}
                      isLoading={isLoading}
                      onChange={setValues}
                      onSubmit={submitDatabase}
                    />
                  ) : (
                    <AdminStep
                      values={values}
                      fields={fields}
                      isLoading={isLoading}
                      onChange={setValues}
                      showToken={mode === "ADMIN_ONLY"}
                      onBack={
                        mode === "FULL"
                          ? () => {
                              if (isLoading) return
                              setFields({})
                              setError(null)
                              setStep("database")
                            }
                          : undefined
                      }
                      onSubmit={submitAdmin}
                    />
                  )}
                </MountTransition>
              </Stack>
            </Section>
          </Card>
        </Center>
      </div>
    </AppShell>
  )
}

function mergeApiFields(
  cause: unknown,
  setFields: (value: Record<string, string>) => void,
) {
  if (cause instanceof ApiClientError && cause.payload.fields) {
    setFields(cause.payload.fields)
  }
}
