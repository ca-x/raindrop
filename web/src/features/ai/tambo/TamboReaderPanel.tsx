import { Banner } from "@astryxdesign/core/Banner"
import { Button } from "@astryxdesign/core/Button"
import { Spinner } from "@astryxdesign/core/Spinner"
import { useLingui } from "@lingui/react"
import { Component, lazy, Suspense, useEffect, useId, useRef, useState, type ReactNode } from "react"

const Conversation = lazy(() => import("./TamboConversation"))

export interface TamboArticleContext {
  title: string
  text: string
}

export interface TamboReaderPanelProps {
  csrfToken: string
  origin: string
  article: TamboArticleContext
  onClose: () => void
}

export function TamboReaderPanel(props: TamboReaderPanelProps) {
  const { i18n } = useLingui()
  const [started, setStarted] = useState(false)
  const headingId = useId()
  const heading = useRef<HTMLHeadingElement>(null)
  useEffect(() => { heading.current?.focus({ preventScroll: true }) }, [])
  return (
    <section className="reader-ai-sidecar tambo-panel" data-started={started} aria-labelledby={headingId} onKeyDown={(event) => {
      if (event.key === "Escape") { event.stopPropagation(); props.onClose() }
    }}>
      <div className="reader-ai-sidecar-header">
        <h2 id={headingId} ref={heading} tabIndex={-1}>{i18n._("tambo.title")}</h2>
        <Button label={i18n._("ai.reader.close")} onClick={props.onClose} variant="ghost" />
      </div>
      <div className="reader-ai-sidecar-content">
        {!started ? <div className="tambo-introduction">
          <p>{i18n._("tambo.introduction")}</p>
          <p className="reader-preference-description">{i18n._("tambo.consent", { origin: props.origin })}</p>
          <Button label={i18n._("tambo.start")} variant="primary" onClick={() => {
            heading.current?.focus({ preventScroll: true })
            setStarted(true)
          }} />
        </div> : <ConversationBoundary fallback={
          <Banner status="warning" title={i18n._("tambo.unavailable")} description={i18n._("tambo.fallbackDescription")}
            endContent={<Button label={i18n._("common.retry")} variant="secondary" onClick={() => {
              heading.current?.focus({ preventScroll: true })
              setStarted(false)
            }} />} />
        }>
          <Suspense fallback={<Spinner label={i18n._("tambo.loading")} />}>
            <Conversation csrfToken={props.csrfToken} article={props.article} />
          </Suspense>
        </ConversationBoundary>}
      </div>
    </section>
  )
}

class ConversationBoundary extends Component<{ children: ReactNode; fallback: ReactNode }, { failed: boolean }> {
  state = { failed: false }
  static getDerivedStateFromError() { return { failed: true } }
  render() { return this.state.failed ? this.props.fallback : this.props.children }
}
