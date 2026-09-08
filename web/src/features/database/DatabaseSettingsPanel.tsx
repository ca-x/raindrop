import { AlertDialog } from "@astryxdesign/core/AlertDialog"
import { NumberInput } from "@astryxdesign/core/NumberInput"
import { Switch } from "@astryxdesign/core/Switch"
import { Button } from "@astryxdesign/core/Button"
import { useLingui } from "@lingui/react"
import { useEffect, useRef, useState } from "react"
import { ApiClientError } from "../../shared/api/client"
import { databaseRequest, saveArticleRetention, type ArticleRetentionSettings, type MaintenanceStatus } from "./api"

export function DatabaseSettingsPanel({ csrfToken, onUnauthenticated }: {
  csrfToken: string
  onUnauthenticated?: () => void
}) {
  const { i18n } = useLingui()
  const [status, setStatus] = useState<MaintenanceStatus | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [pending, setPending] = useState(false)
  const [reload, setReload] = useState(0)
  const mounted = useRef(true)
  const submitting = useRef(false)
  const unauthenticated = useRef(onUnauthenticated)
  unauthenticated.current = onUnauthenticated

  const handleError = (cause: unknown) => {
    if (cause instanceof ApiClientError && cause.status === 401) unauthenticated.current?.()
    setError(cause instanceof ApiClientError && cause.payload.code === "DATABASE_UNSUPPORTED"
      ? "database.unsupported" : "database.error")
  }
  useEffect(() => {
    mounted.current = true
    let cancelled = false
    let timer: ReturnType<typeof setTimeout> | undefined
    async function poll() {
      try {
        const next = await databaseRequest()
        if (cancelled) return
        setStatus(next)
        setError(null)
        if (next.running) timer = setTimeout(() => void poll(), 2000)
      } catch (cause) {
        if (!cancelled) handleError(cause)
      }
    }
    void poll()
    return () => { cancelled = true; mounted.current = false; clearTimeout(timer) }
  }, [reload])

  const compact = async () => {
    if (submitting.current || status?.running) return
    submitting.current = true
    setPending(true)
    setError(null)
    try {
      const next = await databaseRequest(csrfToken)
      if (mounted.current) { setStatus(next); setReload((value) => value + 1) }
    } catch (cause) {
      if (mounted.current) handleError(cause)
    } finally {
      submitting.current = false
      if (mounted.current) setPending(false)
    }
  }
  const bytes = (value: number) => `${(value / 1024 / 1024).toLocaleString(i18n.locale, { maximumFractionDigits: 2 })} MiB`
  const storage = status?.storage
  const running = pending || status?.running
  return <section className="reader-database-settings" aria-busy={Boolean(running)}>
    <div className="reader-settings-panel-intro">
      <div className="reader-settings-title">{i18n._("database.title")}</div>
      <p className="reader-preference-description">{i18n._("database.description")}</p>
    </div>
    {error ? <div role="alert">
      <p>{i18n._(error)}</p>
      {error !== "database.unsupported" ? <Button label={i18n._("common.retry")} variant="secondary"
        onClick={() => { setError(null); setReload((value) => value + 1) }} /> : null}
    </div> : null}
    {storage ? <>
      <dl className="reader-database-stats">
        {([
          ["database.size", bytes(storage.databaseBytes)],
          ["database.wal", bytes(storage.walBytes)],
          ["database.free", bytes(storage.freeBytes)],
          ["database.entries", storage.entryCount.toLocaleString(i18n.locale)],
          ["database.feeds", storage.feedCount.toLocaleString(i18n.locale)],
        ] as const).map(([key, value]) => <div key={key}><dt>{i18n._(key)}</dt><dd>{value}</dd></div>)}
      </dl>
      <p className="reader-preference-description">{i18n._("database.countHint")}</p>
      {storage.tables ? <details>
        <summary>{i18n._("database.breakdown")}</summary>
        <dl className="reader-database-stats">{storage.tables.map((table) =>
          <div key={table.name}><dt>{table.name}</dt><dd>{bytes(table.bytes)}</dd></div>)}</dl>
      </details> : null}
    </> : !error ? <p role="status">{i18n._("database.loading")}</p> : null}
    {status ? <ArticleRetentionForm csrfToken={csrfToken} settings={status.articleRetention}
      onSave={(settings) => setStatus((current) => current ? { ...current, articleRetention: settings } : current)}
      onUnauthenticated={onUnauthenticated} /> : null}
    {storage ? <div className="reader-database-section">
      <div className="reader-preference-label">{i18n._("database.compact")}</div>
      <p className="reader-preference-description">{i18n._("database.compactHint")}</p>
      <Button label={i18n._(running ? "database.compacting" : "database.compact")}
        variant="secondary" isDisabled={Boolean(running)} isLoading={Boolean(running)} onClick={() => void compact()} />
    </div> : null}
    <div role="status" aria-live="polite">
      {status?.running ? <p>{i18n._("database.runningHint")}</p> : null}
      {status?.result ? <p>{i18n._("database.success", {
        before: bytes(status.result.before.databaseBytes + status.result.before.walBytes),
        after: bytes(status.result.after.databaseBytes + status.result.after.walBytes),
        size: bytes(status.result.reclaimedBytes),
      })}</p> : null}
      {status?.result && !status.result.walTruncated ? <p>{i18n._("database.walBusy")}</p> : null}
    </div>
    {status?.error ? <p role="alert">{i18n._("database.compactionError")}</p> : null}
  </section>
}

function ArticleRetentionForm({ csrfToken, settings, onSave, onUnauthenticated }: {
  csrfToken: string
  settings: ArticleRetentionSettings
  onSave: (settings: ArticleRetentionSettings) => void
  onUnauthenticated?: () => void
}) {
  const { i18n } = useLingui()
  const [enabled, setEnabled] = useState(settings.enabled)
  const [days, setDays] = useState<number | undefined>(settings.retentionDays)
  const [saving, setSaving] = useState(false)
  const [confirming, setConfirming] = useState(false)
  const [error, setError] = useState(false)
  const [saved, setSaved] = useState(false)
  const dirty = enabled !== settings.enabled || days !== settings.retentionDays
  const valid = days !== undefined && Number.isInteger(days) && days >= 1 && days <= 3650
  const save = async () => {
    if (saving || !valid) return
    setSaving(true)
    setError(false)
    try {
      const value = await saveArticleRetention({ enabled, retentionDays: days! }, csrfToken)
      onSave(value)
      setSaved(true)
      setConfirming(false)
    } catch (cause) {
      if (cause instanceof ApiClientError && cause.status === 401) onUnauthenticated?.()
      setError(true)
      setConfirming(false)
    } finally { setSaving(false) }
  }
  return <div className="reader-database-section">
    <div className="reader-database-cleanup-heading">
      <div>
        <div className="reader-preference-label">{i18n._("database.articleCleanup")}</div>
        <p className="reader-preference-description">{i18n._("database.articleCleanupHint")}</p>
      </div>
      <Switch label={i18n._("database.autoCleanup")} value={enabled} isDisabled={saving}
        onChange={(value) => { setEnabled(value); setSaved(false) }} />
    </div>
    <NumberInput label={i18n._("database.retentionDays")} value={days} onChange={(value) => { setDays(value); setSaved(false) }}
      min={1} max={3650} isIntegerOnly isDisabled={!enabled || saving} width="min(100%, 240px)"
      description={i18n._("database.retentionHint")} />
    <p className="reader-preference-description">{i18n._("database.protectionHint")}</p>
    <Button label={i18n._("preferences.save")} variant="secondary" isDisabled={!dirty || !valid || saving} isLoading={saving}
      onClick={() => {
        if (enabled && (!settings.enabled || days! < settings.retentionDays)) setConfirming(true)
        else void save()
      }} />
    {error ? <p role="alert">{i18n._("database.settingsError")}</p> : null}
    <div role="status" aria-live="polite">{saved && !dirty ? i18n._("database.settingsSaved") : null}</div>
    <AlertDialog isOpen={confirming} onOpenChange={(open) => { if (!saving) setConfirming(open) }}
      title={i18n._("database.confirmCleanup")} description={i18n._("database.confirmCleanupHint", { days: days ?? 30 })}
      actionLabel={i18n._("database.enableCleanup")} cancelLabel={i18n._("common.cancel")}
      isActionLoading={saving} onAction={() => void save()} />
  </div>
}
