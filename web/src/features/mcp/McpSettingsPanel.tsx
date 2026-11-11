import { AlertDialog } from "@astryxdesign/core/AlertDialog"
import { Button } from "@astryxdesign/core/Button"
import { Selector } from "@astryxdesign/core/Selector"
import { TextInput } from "@astryxdesign/core/TextInput"
import { useLingui } from "@lingui/react"
import { useEffect, useRef, useState, type FormEvent } from "react"
import { ApiClientError } from "../../shared/api/client"
import { createMcpToken, listMcpTokens, revokeMcpToken, type CreatedMcpToken, type McpToken } from "./api"
import "./mcp.css"

export function McpSettingsPanel({ csrfToken, onUnauthenticated }: {
  csrfToken?: string
  onUnauthenticated?: () => void
}) {
  const { i18n } = useLingui()
  const [tokens, setTokens] = useState<McpToken[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [name, setName] = useState("")
  const [days, setDays] = useState("90")
  const [created, setCreated] = useState<CreatedMcpToken | null>(null)
  const [isCreating, setIsCreating] = useState(false)
  const [revoking, setRevoking] = useState<McpToken | null>(null)
  const [pending, setPending] = useState(false)
  const [message, setMessage] = useState<string | null>(null)
  const [reload, setReload] = useState(0)
  const mounted = useRef(false)
  const busy = useRef(false)
  const secretField = useRef<HTMLTextAreaElement>(null)
  const nameField = useRef<HTMLInputElement>(null)
  const createButton = useRef<HTMLButtonElement>(null)
  const unauthenticated = useRef(onUnauthenticated)
  unauthenticated.current = onUnauthenticated

  const handleError = (cause: unknown) => {
    if (cause instanceof ApiClientError && cause.status === 401) {
      setCreated(null)
      unauthenticated.current?.()
    }
    setError(cause instanceof ApiClientError && (cause.status === 429 || cause.payload.code === "MCP_TOKEN_LIMIT")
      ? "mcp.limitError" : "mcp.error")
  }

  useEffect(() => {
    mounted.current = true
    let cancelled = false
    void listMcpTokens().then((items) => {
      if (!cancelled) { setTokens(items); setError(null) }
    }).catch((cause: unknown) => { if (!cancelled) handleError(cause) })
    return () => { cancelled = true; mounted.current = false }
  }, [reload])

  useEffect(() => {
    if (created) secretField.current?.focus()
    else if (nameField.current) nameField.current.focus()
    else createButton.current?.focus()
  }, [created, isCreating])

  const create = async (event: FormEvent) => {
    event.preventDefault()
    if (busy.current || !csrfToken || !name.trim() || name.trim().length > 80 || created) return
    busy.current = true
    setPending(true)
    setError(null)
    setMessage(null)
    try {
      const result = await createMcpToken(name.trim(), Number(days), csrfToken)
      if (mounted.current) {
        setCreated(result)
        setIsCreating(false)
        setTokens((items) => [result.item, ...(items ?? [])])
        setName("")
      }
    } catch (cause) { if (mounted.current) handleError(cause) }
    finally { busy.current = false; if (mounted.current) setPending(false) }
  }

  const revoke = async () => {
    if (busy.current || !csrfToken || !revoking) return
    busy.current = true
    setPending(true)
    setError(null)
    setMessage(null)
    try {
      await revokeMcpToken(revoking.id, csrfToken)
      if (mounted.current) {
        setTokens((items) => items?.filter((item) => item.id !== revoking.id) ?? [])
        if (created?.item.id === revoking.id) setCreated(null)
        setRevoking(null)
        setMessage("mcp.revoked")
      }
    } catch (cause) { if (mounted.current) handleError(cause) }
    finally { busy.current = false; if (mounted.current) setPending(false) }
  }

  const endpoint = new URL("/mcp", window.location.origin).href
  const configuration = JSON.stringify({ mcpServers: { raindrop: {
    type: "http", url: endpoint, headers: { Authorization: "Bearer <YOUR_TOKEN>" },
  } } }, null, 2)
  const date = (value: string) => new Date(value).toLocaleDateString(i18n.locale)

  return <section className="reader-mcp-settings" aria-busy={pending}>
    <div className="reader-mcp-boundary">
      <strong>{i18n._("mcp.readOnly")}</strong>
      <p className="reader-preference-description">{i18n._("mcp.boundary")}</p>
    </div>
    {error ? <div role="alert">
      <p>{i18n._(error)}</p>
      {tokens === null ? <Button label={i18n._("common.retry")} variant="secondary"
        onClick={() => setReload((value) => value + 1)} /> : null}
    </div> : null}
    {tokens === null && !error ? <p role="status">{i18n._("mcp.loading")}</p> : null}
    {created ? <section className="reader-mcp-secret" aria-labelledby="mcp-secret-title">
      <h4 id="mcp-secret-title">{i18n._("mcp.secretTitle")}</h4>
      <p id="mcp-secret-warning" className="reader-preference-description">{i18n._("mcp.secretWarning")}</p>
      <textarea ref={secretField} aria-label={i18n._("mcp.token")} aria-describedby="mcp-secret-warning"
        value={created.token} readOnly rows={3} spellCheck={false} autoComplete="off"
        onFocus={(event) => event.currentTarget.select()} />
      <div className="reader-mcp-actions">
        <Button label={i18n._("mcp.copyToken")} variant="secondary" onClick={() => {
          void navigator.clipboard?.writeText(created.token).then(() => {
            if (mounted.current) setMessage("mcp.copied")
          }).catch(() => { if (mounted.current) setMessage("mcp.copyFallback") })
          if (!navigator.clipboard) setMessage("mcp.copyFallback")
        }} />
        <Button label={i18n._("mcp.saved")} variant="ghost" onClick={() => { setCreated(null); setMessage(null) }} />
      </div>
    </section> : tokens !== null && (tokens.length === 0 || isCreating) ? <form className="reader-mcp-create" onSubmit={(event) => void create(event)}>
      <TextInput ref={nameField} label={i18n._("mcp.name")} value={name} onChange={setName}
        width="100%" isRequired isDisabled={pending || !csrfToken} />
      <Selector label={i18n._("mcp.expiry")} value={days} onChange={setDays} width="100%" isDisabled={pending}
        options={[30, 90, 365].map((value) => ({ value: String(value), label: i18n._("mcp.days", { days: value }) }))} />
      <Button label={i18n._("mcp.create")} type="submit" variant="secondary" isLoading={pending}
        isDisabled={pending || !csrfToken || !name.trim() || name.trim().length > 80 || tokens.length >= 50} />
      {tokens.length > 0 ? <Button label={i18n._("common.cancel")} variant="ghost" isDisabled={pending}
        onClick={() => setIsCreating(false)} /> : null}
    </form> : tokens !== null ? <div><Button ref={createButton} label={i18n._("mcp.create")} variant="secondary"
      isDisabled={pending || !csrfToken || tokens.length >= 50} onClick={() => setIsCreating(true)} /></div> : null}
    <div role="status" aria-live="polite">{message ? i18n._(message) : null}</div>
    {tokens !== null ? <section aria-labelledby="mcp-tokens-heading">
      <h4 id="mcp-tokens-heading">{i18n._("mcp.tokens")}</h4>
      {tokens.length === 0 ? <p className="reader-preference-description">{i18n._("mcp.empty")}</p> :
        <ul className="reader-mcp-tokens">{tokens.map((token) => <li key={token.id}>
          <div className="reader-mcp-token-info">
            <strong>{token.name}</strong>
            <code>{token.prefix}…</code>
            <span className="reader-preference-description">{i18n._("mcp.expires", { date: date(token.expiresAt) })}
              {Date.parse(token.expiresAt) <= Date.now() ? ` · ${i18n._("mcp.expired")}` : ""}</span>
            <span className="reader-preference-description">{token.lastUsedAt
              ? i18n._("mcp.lastUsed", { date: date(token.lastUsedAt) }) : i18n._("mcp.neverUsed")}</span>
          </div>
          <Button label={i18n._("mcp.revoke")} aria-label={i18n._("mcp.revokeNamed", { name: token.name })}
            variant="ghost" isDisabled={pending} onClick={() => setRevoking(token)} />
        </li>)}</ul>}
    </section> : null}
    <details className="reader-mcp-connection">
      <summary>{i18n._("mcp.connect")}</summary>
      <p className="reader-preference-description">{i18n._("mcp.connectHint")}</p>
      <code className="reader-mcp-endpoint">{endpoint}</code>
      <pre tabIndex={0} aria-label={i18n._("mcp.config")}>{configuration}</pre>
      <p className="reader-preference-description">{i18n._("mcp.securityHint")}</p>
    </details>
    <AlertDialog isOpen={Boolean(revoking)} onOpenChange={(open) => { if (!open && !busy.current) setRevoking(null) }}
      title={i18n._("mcp.revokeTitle")} description={i18n._("mcp.revokeDescription", { name: revoking?.name ?? "" })}
      actionLabel={i18n._("mcp.revoke")} cancelLabel={i18n._("common.cancel")} isActionLoading={pending}
      onAction={() => void revoke()} />
  </section>
}
