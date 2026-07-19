import { Badge } from "@astryxdesign/core/Badge"
import { Banner } from "@astryxdesign/core/Banner"
import { Button } from "@astryxdesign/core/Button"
import { FileInput } from "@astryxdesign/core/FileInput"
import { Grid } from "@astryxdesign/core/Grid"
import { Stack } from "@astryxdesign/core/Stack"
import { Text } from "@astryxdesign/core/Text"
import { useLingui } from "@lingui/react"
import { useEffect, useRef, useState } from "react"

import { ApiClientError } from "../../../shared/api/client"
import * as defaultApi from "../api"
import type { OpmlImportSummary } from "../model"

const MAX_OPML_BYTES = 10 * 1024 * 1024

export interface OpmlTransferApi {
  previewOpml: typeof defaultApi.previewOpml
  commitOpml: typeof defaultApi.commitOpml
  exportOpml: typeof defaultApi.exportOpml
}

interface OpmlTransferPanelProps {
  csrfToken: string
  onImported: () => Promise<void> | void
  api?: OpmlTransferApi
}

export function OpmlTransferPanel({
  csrfToken,
  onImported,
  api = defaultApi,
}: OpmlTransferPanelProps) {
  const { i18n } = useLingui()
  const [file, setFile] = useState<File | null>(null)
  const [summary, setSummary] = useState<OpmlImportSummary | null>(null)
  const [phase, setPhase] = useState<"idle" | "preview" | "import" | "export">("idle")
  const [error, setError] = useState<string | null>(null)
  const [success, setSuccess] = useState<string | null>(null)
  const previewRequest = useRef<AbortController | null>(null)

  useEffect(() => () => previewRequest.current?.abort(), [])

  const selectFile = (selected: File | File[] | null) => {
    const nextFile = selected instanceof File ? selected : null
    previewRequest.current?.abort()
    setFile(nextFile)
    setSummary(null)
    setError(null)
    setSuccess(null)
    if (!nextFile) {
      setPhase("idle")
      return
    }
    const request = new AbortController()
    previewRequest.current = request
    setPhase("preview")
    void api
      .previewOpml(nextFile, csrfToken, request.signal)
      .then((preview) => {
        if (request.signal.aborted) return
        setSummary(preview)
        setPhase("idle")
      })
      .catch((cause: unknown) => {
        if (request.signal.aborted) return
        setError(errorMessage(cause, i18n._("opml.previewError")))
        setPhase("idle")
      })
  }

  const importFile = async () => {
    if (!file || phase !== "idle") return
    setError(null)
    setSuccess(null)
    setPhase("import")
    try {
      const imported = await api.commitOpml(file, csrfToken)
      setSummary(imported)
      setSuccess(
        i18n._("opml.importSuccess", { count: imported.importedCount }),
      )
      await onImported()
    } catch (cause) {
      setError(errorMessage(cause, i18n._("opml.importError")))
    } finally {
      setPhase("idle")
    }
  }

  const exportFile = async () => {
    if (phase !== "idle") return
    setError(null)
    setSuccess(null)
    setPhase("export")
    try {
      const exported = await api.exportOpml()
      downloadBlob(exported.blob, exported.filename ?? "raindrop.opml")
      setSuccess(i18n._("opml.exportSuccess"))
    } catch (cause) {
      setError(errorMessage(cause, i18n._("opml.exportError")))
    } finally {
      setPhase("idle")
    }
  }

  return (
    <Stack gap={5} className="reader-opml-transfer">
      <Stack gap={1}>
        <Text type="large" weight="semibold">
          {i18n._("opml.title")}
        </Text>
        <Text type="supporting" color="secondary" textWrap="pretty">
          {i18n._("opml.description")}
        </Text>
      </Stack>
      {error ? <Banner status="error" title={error} /> : null}
      {success ? <Banner status="success" title={success} /> : null}
      <FileInput
        label={i18n._("opml.file")}
        description={i18n._("opml.fileDescription")}
        placeholder={i18n._("opml.chooseFile")}
        value={file}
        onChange={selectFile}
        accept=".opml,.xml,application/xml,text/xml"
        maxSize={MAX_OPML_BYTES}
        mode="dropzone"
        isLoading={phase === "preview"}
        isDisabled={phase === "import" || phase === "export"}
      />
      {summary ? <OpmlSummary summary={summary} /> : null}
      <div className="reader-opml-actions">
        <Button
          label={i18n._("opml.export")}
          variant="secondary"
          isLoading={phase === "export"}
          isDisabled={phase !== "idle"}
          onClick={exportFile}
        />
        <Button
          label={i18n._("opml.import")}
          variant="primary"
          isLoading={phase === "import"}
          isDisabled={!file || !summary || phase !== "idle"}
          onClick={importFile}
        />
      </div>
    </Stack>
  )
}

function OpmlSummary({ summary }: { summary: OpmlImportSummary }) {
  const { i18n } = useLingui()
  const values = [
    ["opml.newCount", summary.newCount, "success"],
    ["opml.duplicateCount", summary.duplicateCount, "neutral"],
    ["opml.invalidCount", summary.invalidCount, summary.invalidCount ? "warning" : "neutral"],
    ["opml.categoryCount", summary.categoryCount, "info"],
  ] as const
  return (
    <Grid columns={{ minWidth: 128, max: 4 }} gap={2}>
      {values.map(([label, value, variant]) => (
        <div className="reader-opml-stat" key={label}>
          <Text type="supporting" color="secondary">
            {i18n._(label)}
          </Text>
          <Badge label={String(value)} variant={variant} />
        </div>
      ))}
    </Grid>
  )
}

function errorMessage(cause: unknown, fallback: string): string {
  if (cause instanceof ApiClientError) {
    return cause.payload.fields?.file ?? cause.payload.message ?? fallback
  }
  return fallback
}

function downloadBlob(blob: Blob, filename: string) {
  const url = URL.createObjectURL(blob)
  const anchor = document.createElement("a")
  anchor.href = url
  anchor.download = filename
  anchor.click()
  URL.revokeObjectURL(url)
}
