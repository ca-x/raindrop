import { AlertDialog } from "@astryxdesign/core/AlertDialog"
import { Banner } from "@astryxdesign/core/Banner"
import { Button } from "@astryxdesign/core/Button"
import { CheckboxInput } from "@astryxdesign/core/CheckboxInput"
import { Icon } from "@astryxdesign/core/Icon"
import { IconButton } from "@astryxdesign/core/IconButton"
import { NumberInput } from "@astryxdesign/core/NumberInput"
import { Switch } from "@astryxdesign/core/Switch"
import { Tab, TabList } from "@astryxdesign/core/TabList"
import { TextInput } from "@astryxdesign/core/TextInput"
import { Tooltip } from "@astryxdesign/core/Tooltip"
import { useLingui } from "@lingui/react"
import { useEffect, useMemo, useState, type FormEvent, type ReactNode } from "react"

import type {
  BackupCredentials,
  BackupJobStatus,
  BackupTarget,
  BackupTargetKind,
  RetentionPolicy,
  SaveBackupTargetRequest,
} from "../api/backup.generated"
import type { BackupController } from "../model/useBackupController"

type BackupTab = "s3" | "webdav" | "schedule" | "history"

export function BackupSettingsPanel({ controller }: { controller: BackupController }) {
  const { i18n } = useLingui()
  const [activeTab, setActiveTab] = useState<BackupTab>("s3")
  const [form, setForm] = useState<{ kind: BackupTargetKind; target: BackupTarget | null } | null>(null)
  const [deleteTarget, setDeleteTarget] = useState<BackupTarget | null>(null)
  const [testedTargetId, setTestedTargetId] = useState<string | null>(null)

  if (controller.loadStatus === "loading" || controller.loadStatus === "idle") {
    return <div className="reader-backup-loading">{i18n._("backup.loading")}</div>
  }

  return (
    <div className="reader-backup-settings">
      <div className="reader-settings-panel-intro">
        <div className="reader-settings-title">{i18n._("backup.title")}</div>
        <div className="reader-preference-description">
          {i18n._("backup.description")}
        </div>
      </div>
      {controller.error ? (
        <Banner
          status="error"
          title={i18n._("backup.errorTitle")}
          description={i18n._(`backup.error.${controller.error}`)}
        />
      ) : null}
      {testedTargetId ? (
        <Banner
          status="success"
          title={i18n._("backup.testSuccess")}
          description={i18n._("backup.testSuccessDescription")}
        />
      ) : null}
      <div className="reader-backup-tabs">
        <TabList
          value={activeTab}
          onChange={(value) => {
            setActiveTab(value as BackupTab)
            setForm(null)
            controller.clearError()
          }}
          hasDivider
        >
          <Tab value="s3" label="S3" />
          <Tab value="webdav" label="WebDAV" />
          <Tab value="schedule" label={i18n._("backup.scheduleTab")} />
          <Tab value="history" label={i18n._("backup.historyTab")} />
        </TabList>
      </div>
      <div key={activeTab} className="reader-backup-panel reader-panel-transition">
        {activeTab === "s3" || activeTab === "webdav" ? (
          form ? (
            <TargetForm
              kind={form.kind}
              target={form.target}
              isSaving={controller.isMutating}
              onCancel={() => setForm(null)}
              onSave={async (request, credentials) => {
                const saved = form.target
                  ? await controller.updateTarget(form.target.targetId, {
                      ...request,
                      ...(credentials ? { credentials } : {}),
                    })
                  : credentials
                    ? await controller.createTarget({ ...request, credentials })
                    : false
                if (saved) setForm(null)
              }}
            />
          ) : (
            <TargetList
              kind={activeTab === "s3" ? "S3" : "WEBDAV"}
              targets={controller.targets}
              isMutating={controller.isMutating}
              activeTargetId={controller.activeTargetId}
              onAdd={(kind) => setForm({ kind, target: null })}
              onEdit={(target) => setForm({ kind: target.config.kind, target })}
              onDelete={setDeleteTarget}
              onTest={async (targetId) => {
                setTestedTargetId(null)
                if (await controller.testTarget(targetId)) setTestedTargetId(targetId)
              }}
              onEnabledChange={(target, enabled) => controller.updateTarget(
                target.targetId,
                targetRequest(target, enabled),
              )}
            />
          )
        ) : activeTab === "schedule" ? (
          <SchedulePanel controller={controller} />
        ) : (
          <HistoryPanel controller={controller} />
        )}
      </div>
      <AlertDialog
        isOpen={Boolean(deleteTarget)}
        onOpenChange={(open) => {
          if (!controller.isMutating && !open) setDeleteTarget(null)
        }}
        title={i18n._("backup.deleteTitle")}
        description={i18n._("backup.deleteDescription", {
          name: deleteTarget?.displayName ?? "",
        })}
        actionLabel={i18n._("backup.deleteAction")}
        cancelLabel={i18n._("common.cancel")}
        isActionLoading={controller.isMutating}
        onAction={() => {
          if (!deleteTarget) return
          void controller.deleteTarget(deleteTarget.targetId).then((deleted) => {
            if (deleted) setDeleteTarget(null)
          })
        }}
      />
    </div>
  )
}

function TargetList(props: {
  kind: BackupTargetKind
  targets: BackupTarget[]
  isMutating: boolean
  activeTargetId: string | null
  onAdd: (kind: BackupTargetKind) => void
  onEdit: (target: BackupTarget) => void
  onDelete: (target: BackupTarget) => void
  onTest: (targetId: string) => Promise<void>
  onEnabledChange: (target: BackupTarget, enabled: boolean) => Promise<boolean>
}) {
  const { i18n } = useLingui()
  const targets = props.targets.filter((target) => target.config.kind === props.kind)
  return (
    <section className="reader-backup-section">
      <div className="reader-backup-section-header">
        <div>
          <div className="reader-preference-label">{props.kind}</div>
          <div className="reader-preference-description">
            {i18n._(props.kind === "S3" ? "backup.s3Description" : "backup.webdavDescription")}
          </div>
        </div>
        <Button
          label={i18n._("backup.addTarget", { kind: props.kind })}
          onClick={() => props.onAdd(props.kind)}
          variant="primary"
        />
      </div>
      {targets.length === 0 ? (
        <div className="reader-backup-empty">
          <Icon icon="info" size="md" color="secondary" />
          <div>
            <div className="reader-preference-label">{i18n._("backup.emptyTargets")}</div>
            <div className="reader-preference-description">
              {i18n._("backup.emptyTargetsDescription", { kind: props.kind })}
            </div>
          </div>
        </div>
      ) : (
        <div className="reader-backup-target-list">
          {targets.map((target) => (
            <article key={target.targetId} className="reader-backup-target-card">
              <div className="reader-backup-target-main">
                <div className="reader-backup-target-heading">
                  <span className="reader-preference-label">{target.displayName}</span>
                  <span className="reader-backup-kind">{target.config.kind}</span>
                </div>
                <div className="reader-backup-endpoint">{target.config.settings.endpoint}</div>
                <div className="reader-preference-description">
                  {retentionSummary(target.retention, i18n._.bind(i18n))}
                </div>
              </div>
              <div className="reader-backup-target-actions">
                <Switch
                  label={i18n._("backup.enabled")}
                  isLabelHidden
                  value={target.enabled}
                  isLoading={props.activeTargetId === target.targetId && props.isMutating}
                  isDisabled={props.isMutating && props.activeTargetId !== target.targetId}
                  onChange={(enabled) => void props.onEnabledChange(target, enabled)}
                />
                <ActionIcon
                  label={i18n._("backup.testTarget", { name: target.displayName })}
                  icon={<PulseIcon />}
                  isDisabled={props.isMutating}
                  onClick={() => void props.onTest(target.targetId)}
                />
                <ActionIcon
                  label={i18n._("backup.editTarget", { name: target.displayName })}
                  icon={<EditIcon />}
                  isDisabled={props.isMutating}
                  onClick={() => props.onEdit(target)}
                />
                <ActionIcon
                  label={i18n._("backup.deleteTarget", { name: target.displayName })}
                  icon={<TrashIcon />}
                  isDisabled={props.isMutating}
                  onClick={() => props.onDelete(target)}
                  variant="destructive"
                />
              </div>
            </article>
          ))}
        </div>
      )}
    </section>
  )
}

function TargetForm(props: {
  kind: BackupTargetKind
  target: BackupTarget | null
  isSaving: boolean
  onCancel: () => void
  onSave: (request: SaveBackupTargetRequest, credentials: BackupCredentials | null) => Promise<void>
}) {
  const { i18n } = useLingui()
  const settings = props.target?.config.settings
  const [displayName, setDisplayName] = useState(props.target?.displayName ?? "")
  const [enabled, setEnabled] = useState(props.target?.enabled ?? true)
  const [endpoint, setEndpoint] = useState(settings?.endpoint ?? "")
  const [prefix, setPrefix] = useState(settings?.prefix ?? "")
  const [region, setRegion] = useState(
    props.target?.config.kind === "S3" ? props.target.config.settings.region : "us-east-1",
  )
  const [bucket, setBucket] = useState(
    props.target?.config.kind === "S3" ? props.target.config.settings.bucket : "",
  )
  const [pathStyle, setPathStyle] = useState(
    props.target?.config.kind === "S3" ? props.target.config.settings.pathStyle : true,
  )
  const [accessKeyId, setAccessKeyId] = useState("")
  const [secretAccessKey, setSecretAccessKey] = useState("")
  const [sessionToken, setSessionToken] = useState("")
  const [username, setUsername] = useState("")
  const [password, setPassword] = useState("")
  const [retainCount, setRetainCount] = useState<number | null>(
    props.target?.retention.retainCount ?? null,
  )
  const [retainDays, setRetainDays] = useState<number | null>(
    props.target?.retention.retainDays ?? null,
  )

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    const config = props.kind === "S3"
      ? { kind: "S3" as const, settings: { endpoint, region, bucket, prefix, pathStyle } }
      : { kind: "WEBDAV" as const, settings: { endpoint, prefix } }
    let credentials: BackupCredentials | null = null
    if (props.kind === "S3" && (accessKeyId || secretAccessKey || sessionToken || !props.target)) {
      credentials = {
        kind: "S3",
        values: {
          accessKeyId,
          secretAccessKey,
          sessionToken: sessionToken || null,
        },
      }
    }
    if (props.kind === "WEBDAV" && (username || password || !props.target)) {
      credentials = { kind: "WEBDAV", values: { username, password } }
    }
    void props.onSave({
      displayName,
      enabled,
      config,
      retention: { retainCount, retainDays },
    }, credentials)
  }

  return (
    <form className="reader-backup-target-form" onSubmit={submit}>
      <div className="reader-backup-form-heading">
        <div>
          <div className="reader-settings-title">
            {i18n._(props.target ? "backup.editTargetTitle" : "backup.addTargetTitle", {
              kind: props.kind,
            })}
          </div>
          <div className="reader-preference-description">
            {i18n._("backup.credentialsHint")}
          </div>
        </div>
        <Switch
          label={i18n._("backup.enabled")}
          value={enabled}
          onChange={setEnabled}
          labelPosition="start"
        />
      </div>
      <div className="reader-backup-form-grid">
        <TextInput
          label={i18n._("backup.displayName")}
          value={displayName}
          onChange={setDisplayName}
          width="100%"
          isRequired
        />
        <TextInput
          label={i18n._("backup.endpoint")}
          description={i18n._("backup.httpsOnly")}
          value={endpoint}
          onChange={setEndpoint}
          width="100%"
          isRequired
        />
        {props.kind === "S3" ? (
          <>
            <TextInput label={i18n._("backup.region")} value={region} onChange={setRegion} width="100%" isRequired />
            <TextInput label={i18n._("backup.bucket")} value={bucket} onChange={setBucket} width="100%" isRequired />
          </>
        ) : null}
        <TextInput
          label={i18n._("backup.prefix")}
          description={i18n._("backup.prefixDescription")}
          value={prefix}
          onChange={setPrefix}
          width="100%"
          isOptional
        />
        {props.kind === "S3" ? (
          <Switch
            label={i18n._("backup.pathStyle")}
            description={i18n._("backup.pathStyleDescription")}
            value={pathStyle}
            onChange={setPathStyle}
          />
        ) : null}
      </div>
      <section className="reader-backup-form-section">
        <div className="reader-preference-label">{i18n._("backup.credentials")}</div>
        {props.target ? (
          <div className="reader-preference-description">
            {i18n._("backup.credentialsConfigured")}
          </div>
        ) : null}
        <div className="reader-backup-form-grid">
          {props.kind === "S3" ? (
            <>
              <TextInput label={i18n._("backup.accessKeyId")} value={accessKeyId} onChange={setAccessKeyId} width="100%" isOptional={Boolean(props.target)} isRequired={!props.target} />
              <TextInput label={i18n._("backup.secretAccessKey")} type="password" value={secretAccessKey} onChange={setSecretAccessKey} width="100%" isOptional={Boolean(props.target)} isRequired={!props.target} />
              <TextInput label={i18n._("backup.sessionToken")} type="password" value={sessionToken} onChange={setSessionToken} width="100%" isOptional />
            </>
          ) : (
            <>
              <TextInput label={i18n._("backup.username")} value={username} onChange={setUsername} width="100%" isOptional={Boolean(props.target)} isRequired={!props.target} />
              <TextInput label={i18n._("backup.password")} type="password" value={password} onChange={setPassword} width="100%" isOptional={Boolean(props.target)} isRequired={!props.target} />
            </>
          )}
        </div>
      </section>
      <section className="reader-backup-form-section">
        <div className="reader-preference-label">{i18n._("backup.retention")}</div>
        <div className="reader-preference-description">{i18n._("backup.retentionDescription")}</div>
        <div className="reader-backup-form-grid">
          <NumberInput
            label={i18n._("backup.retainCount")}
            value={retainCount}
            onChange={setRetainCount}
            min={1}
            max={1000}
            isIntegerOnly
            hasClear
          />
          <NumberInput
            label={i18n._("backup.retainDays")}
            value={retainDays}
            onChange={setRetainDays}
            min={1}
            max={3650}
            isIntegerOnly
            hasClear
          />
        </div>
      </section>
      <div className="reader-backup-form-actions">
        <Button label={i18n._("common.cancel")} onClick={props.onCancel} variant="secondary" isDisabled={props.isSaving} />
        <Button label={i18n._("backup.saveTarget")} type="submit" variant="primary" isLoading={props.isSaving} isDisabled={!displayName.trim() || !endpoint.trim()} />
      </div>
    </form>
  )
}

function SchedulePanel({ controller }: { controller: BackupController }) {
  const { i18n } = useLingui()
  const enabledTargets = controller.targets.filter((target) => target.enabled)
  const [enabled, setEnabled] = useState(controller.schedule.enabled)
  const [intervalHours, setIntervalHours] = useState(controller.schedule.intervalHours)
  const [selected, setSelected] = useState<string[]>(
    controller.schedule.targetIds.filter((id) => enabledTargets.some((target) => target.targetId === id)),
  )

  useEffect(() => {
    setEnabled(controller.schedule.enabled)
    setIntervalHours(controller.schedule.intervalHours)
    setSelected(controller.schedule.targetIds.filter(
      (id) => controller.targets.some((target) => target.enabled && target.targetId === id),
    ))
  }, [controller.schedule.revision, controller.targets])

  const groups = useMemo(() => (["S3", "WEBDAV"] as const).map((kind) => ({
    kind,
    targets: enabledTargets.filter((target) => target.config.kind === kind),
  })), [enabledTargets])
  const toggle = (targetId: string, checked: boolean) => {
    setSelected((current) => checked
      ? [...new Set([...current, targetId])]
      : current.filter((id) => id !== targetId))
  }

  return (
    <section className="reader-backup-section reader-backup-schedule">
      <div className="reader-backup-section-header">
        <div>
          <div className="reader-preference-label">{i18n._("backup.scheduleTitle")}</div>
          <div className="reader-preference-description">{i18n._("backup.scheduleDescription")}</div>
        </div>
        <Switch label={i18n._("backup.enableSchedule")} value={enabled} onChange={setEnabled} labelPosition="start" />
      </div>
      <NumberInput
        label={i18n._("backup.intervalHours")}
        description={i18n._("backup.intervalHoursDescription")}
        value={intervalHours}
        onChange={setIntervalHours}
        min={1}
        max={720}
        isIntegerOnly
        width="min(100%, 320px)"
      />
      <div className="reader-backup-target-picker" aria-label={i18n._("backup.targetSelection")}>
        {groups.map((group) => (
          <section key={group.kind} className="reader-backup-target-group">
            <div className="reader-backup-kind-heading">{group.kind}</div>
            {group.targets.length === 0 ? (
              <div className="reader-preference-description">{i18n._("backup.noEnabledTargets")}</div>
            ) : group.targets.map((target) => (
              <CheckboxInput
                key={target.targetId}
                label={target.displayName}
                description={target.config.settings.endpoint}
                value={selected.includes(target.targetId)}
                onChange={(checked) => toggle(target.targetId, checked)}
              />
            ))}
          </section>
        ))}
      </div>
      {controller.schedule.nextRunAt ? (
        <div className="reader-backup-next-run">
          <Icon icon="calendar" size="sm" color="secondary" />
          {i18n._("backup.nextRun", { time: formatDate(controller.schedule.nextRunAt) })}
        </div>
      ) : null}
      <div className="reader-backup-schedule-actions">
        <Button
          label={i18n._("backup.backupNow")}
          onClick={() => void controller.runNow(selected)}
          isLoading={controller.isMutating}
          isDisabled={selected.length === 0}
          variant="primary"
        />
        <Button
          label={i18n._("backup.saveSchedule")}
          onClick={() => void controller.saveSchedule({ enabled, intervalHours, targetIds: selected })}
          isLoading={controller.isMutating}
          isDisabled={enabled && selected.length === 0}
          variant="secondary"
        />
      </div>
    </section>
  )
}

function HistoryPanel({ controller }: { controller: BackupController }) {
  const { i18n } = useLingui()
  return (
    <section className="reader-backup-section">
      <div className="reader-backup-section-header">
        <div>
          <div className="reader-preference-label">{i18n._("backup.historyTitle")}</div>
          <div className="reader-preference-description">{i18n._("backup.historyDescription")}</div>
        </div>
        <Button label={i18n._("backup.refreshHistory")} onClick={() => void controller.refreshJobs()} variant="secondary" />
      </div>
      {controller.jobs.length === 0 ? (
        <div className="reader-backup-empty">
          <Icon icon="clock" size="md" color="secondary" />
          <div className="reader-preference-description">{i18n._("backup.emptyHistory")}</div>
        </div>
      ) : (
        <div className="reader-backup-history-list">
          {controller.jobs.map((job) => (
            <details key={job.jobId} className="reader-backup-job">
              <summary>
                <StatusBadge status={job.status} />
                <span className="reader-backup-job-label">
                  {i18n._(job.triggerKind === "MANUAL" ? "backup.manualTrigger" : "backup.scheduledTrigger")}
                </span>
                <span className="reader-backup-job-meta">
                  {formatDate(job.createdAt)} · {i18n._("backup.targetCount", { count: job.targetCount })}
                </span>
              </summary>
              <div className="reader-backup-job-targets">
                {job.targets.map((target) => (
                  <div key={target.targetResultId} className="reader-backup-job-target">
                    <div>
                      <div className="reader-preference-label">{target.targetName}</div>
                      <div className="reader-preference-description">{target.targetKind}</div>
                    </div>
                    <div className="reader-backup-job-target-result">
                      <StatusBadge status={target.status} />
                      {target.byteSize !== null ? <span>{formatBytes(target.byteSize)}</span> : null}
                      {target.errorCode ? <code>{target.errorCode}</code> : null}
                    </div>
                  </div>
                ))}
              </div>
            </details>
          ))}
        </div>
      )}
    </section>
  )
}

function StatusBadge({ status }: { status: BackupJobStatus | "QUEUED" | "RUNNING" | "SUCCEEDED" | "FAILED" }) {
  const { i18n } = useLingui()
  return (
    <span className="reader-backup-status" data-status={status}>
      {i18n._(`backup.status.${status}`)}
    </span>
  )
}

function ActionIcon(props: {
  label: string
  icon: ReactNode
  onClick: () => void
  isDisabled: boolean
  variant?: "ghost" | "destructive"
}) {
  return (
    <Tooltip content={props.label} delay={180} hasHoverIndication={false}>
      <IconButton
        label={props.label}
        icon={props.icon}
        onClick={props.onClick}
        isDisabled={props.isDisabled}
        variant={props.variant ?? "ghost"}
        size="sm"
      />
    </Tooltip>
  )
}

function targetRequest(target: BackupTarget, enabled: boolean): SaveBackupTargetRequest {
  return {
    displayName: target.displayName,
    enabled,
    config: target.config,
    retention: target.retention,
  }
}

function retentionSummary(
  retention: RetentionPolicy,
  translate: (id: string, values?: Record<string, unknown>) => string,
): string {
  if (retention.retainCount === null && retention.retainDays === null) {
    return translate("backup.retentionUnlimited")
  }
  return [
    retention.retainCount === null ? null : translate("backup.retentionCountSummary", { count: retention.retainCount }),
    retention.retainDays === null ? null : translate("backup.retentionDaysSummary", { days: retention.retainDays }),
  ].filter(Boolean).join(" · ")
}

function formatDate(value: string): string {
  const date = new Date(value)
  return Number.isNaN(date.getTime()) ? value : new Intl.DateTimeFormat(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(date)
}

function formatBytes(value: number): string {
  return value >= 1024 * 1024
    ? `${(value / 1024 / 1024).toFixed(1)} MiB`
    : `${Math.max(1, Math.round(value / 1024))} KiB`
}

function PulseIcon() {
  return <svg aria-hidden="true" viewBox="0 0 20 20" width="18" height="18" fill="none" stroke="currentColor" strokeWidth="1.7"><path d="M2.5 10h3l1.5-4 3 8 2-5 1.2 1H17.5" strokeLinecap="round" strokeLinejoin="round" /></svg>
}

function EditIcon() {
  return <svg aria-hidden="true" viewBox="0 0 20 20" width="18" height="18" fill="none" stroke="currentColor" strokeWidth="1.7"><path d="m12.8 3.2 4 4L7 17H3v-4Z" strokeLinecap="round" strokeLinejoin="round" /><path d="m10.8 5.2 4 4" /></svg>
}

function TrashIcon() {
  return <svg aria-hidden="true" viewBox="0 0 20 20" width="18" height="18" fill="none" stroke="currentColor" strokeWidth="1.7"><path d="M4 6h12M8 3h4l1 3H7l1-3ZM6 6l.7 11h6.6L14 6M8.5 9v5M11.5 9v5" strokeLinecap="round" strokeLinejoin="round" /></svg>
}
