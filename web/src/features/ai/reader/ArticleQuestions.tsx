import { Banner } from "@astryxdesign/core/Banner"
import { Button } from "@astryxdesign/core/Button"
import { Heading } from "@astryxdesign/core/Heading"
import { Markdown } from "@astryxdesign/core/Markdown"
import { Text } from "@astryxdesign/core/Text"
import { TextArea } from "@astryxdesign/core/TextArea"
import { useLingui } from "@lingui/react"
import { useEffect, useId, useRef, useState, type FormEvent, type KeyboardEvent } from "react"

import { ApiClientError } from "../../../shared/api/client"
import { askArticleQuestion } from "../api/questions"
import type { QuestionTurn } from "../api/questions.generated"
import { safeExternalMarkdownLink } from "./TranslationView"
import "./article-questions.css"

interface ArticleQuestionsProps {
  entryId: string
  summaryJobId: string
  csrfToken: string
  onUnauthenticated?: () => void
}

interface AnswerTurn extends QuestionTurn {
  provider: string
  animate: boolean
}

export function ArticleQuestions(props: ArticleQuestionsProps) {
  return <QuestionSession key={`${props.entryId}:${props.summaryJobId}:${props.csrfToken}`} {...props} />
}

function QuestionSession(props: ArticleQuestionsProps) {
  const { i18n } = useLingui()
  const headingId = useId()
  const [draft, setDraft] = useState("")
  const [turns, setTurns] = useState<AnswerTurn[]>([])
  const [pending, setPending] = useState<string | null>(null)
  const [retryQuestion, setRetryQuestion] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [provider, setProvider] = useState<string | null>(null)
  const active = useRef<AbortController | null>(null)
  const textarea = useRef<HTMLTextAreaElement>(null)
  const keyboardSubmit = useRef(false)
  const draftRef = useRef(draft)
  draftRef.current = draft

  useEffect(() => () => {
    active.current?.abort()
    active.current = null
  }, [props.csrfToken])

  async function submit(event: FormEvent) {
    event.preventDefault()
    await send(draft.trim())
  }

  async function send(question: string) {
    if (active.current || !question || [...question].length > 2000) return
    const abort = new AbortController()
    active.current = abort
    const animate = !keyboardSubmit.current
    keyboardSubmit.current = false
    setError(null)
    setRetryQuestion(null)
    setPending(question)
    if (draftRef.current.trim() === question) setDraft("")
    try {
      const response = await askArticleQuestion(props.entryId, props.csrfToken, {
        summaryJobId: props.summaryJobId,
        question,
        responseLocale: i18n.locale === "zh-CN" ? "zh-CN" : "en",
        history: turns.slice(-8).map(({ question: previousQuestion, answer }) => ({ question: previousQuestion, answer })),
      }, abort.signal)
      if (active.current !== abort) return
      setTurns((previous) => [...previous, { question, answer: response.answerMarkdown, provider: response.providerLabel, animate }])
      setProvider(response.providerLabel)
    } catch (cause) {
      if (active.current !== abort || abort.signal.aborted) return
      if (!draftRef.current) setDraft(question)
      setRetryQuestion(question)
      if (cause instanceof ApiClientError && cause.status === 401) {
        props.onUnauthenticated?.()
        setError("AUTH")
      } else if (cause instanceof ApiClientError && cause.status === 413) setError("SIZE")
      else if (cause instanceof ApiClientError && cause.status === 429) setError("BUSY")
      else if (cause instanceof ApiClientError && cause.status === 409) setError("CONTEXT")
      else if (cause instanceof ApiClientError && cause.status === 504) setError("TIMEOUT")
      else setError("REQUEST")
    } finally {
      if (active.current === abort) {
        active.current = null
        setPending(null)
      }
    }
  }

  function stop() {
    const question = pending
    active.current?.abort()
    active.current = null
    setPending(null)
    if (question && !draftRef.current) setDraft(question)
    else setRetryQuestion(question)
    textarea.current?.focus({ preventScroll: true })
  }

  function keyDown(event: KeyboardEvent<HTMLFormElement>) {
    if (event.key !== "Enter" || (!event.ctrlKey && !event.metaKey)
      || event.nativeEvent.isComposing || event.nativeEvent.keyCode === 229) return
    event.preventDefault()
    keyboardSubmit.current = true
    event.currentTarget.requestSubmit()
  }

  function suggest(key: string) {
    setDraft(i18n._(key))
    textarea.current?.focus({ preventScroll: true })
  }

  return <section className="article-questions" aria-labelledby={headingId}>
    <div className="article-questions-heading">
      <Heading level={3} id={headingId}>{i18n._("ai.questions.title")}</Heading>
      <Text as="p" display="block" color="secondary">{i18n._("ai.questions.description")}</Text>
    </div>
    {retryQuestion && draft.trim() !== retryQuestion ? <div className="article-question-turn">
      <div className="article-question-source"><Text type="supporting" color="secondary">{i18n._("ai.questions.you")}</Text><p>{retryQuestion}</p></div>
      <Button label={i18n._("ai.questions.retry")} variant="secondary" onClick={() => void send(retryQuestion)} />
    </div> : null}
    <div className="article-question-turns" role="log" aria-label={i18n._("ai.questions.conversation")} aria-live="polite" aria-relevant="additions">
      {turns.map((turn, index) => <div className="article-question-turn" key={index}>
        <div className="article-question-source">
          <Text type="supporting" color="secondary">{i18n._("ai.questions.you")}</Text>
          <p>{turn.question}</p>
        </div>
        <div className="article-question-answer" data-animate={turn.animate}>
          <Text type="supporting" color="secondary">{i18n._("ai.questions.answer")}</Text>
          <Markdown headingLevelStart={4} contentWidth="100%" onLinkClick={safeExternalMarkdownLink}>{turn.answer}</Markdown>
        </div>
      </div>)}
      {pending ? <div className="article-question-turn article-question-pending">
        <div className="article-question-source"><Text type="supporting" color="secondary">{i18n._("ai.questions.you")}</Text><p>{pending}</p></div>
        <Text as="p" color="secondary" role="status">{i18n._("ai.questions.responding")}</Text>
      </div> : null}
    </div>
    {!turns.length && !pending ? <div className="article-question-suggestions">
      <Button label={i18n._("ai.questions.suggestConcepts")} variant="ghost" onClick={() => suggest("ai.questions.conceptsPrompt")} />
      <Button label={i18n._("ai.questions.suggestLimits")} variant="ghost" onClick={() => suggest("ai.questions.limitsPrompt")} />
    </div> : null}
    <form className="article-question-composer" onSubmit={(event) => { void submit(event) }} onKeyDown={keyDown}>
      <TextArea ref={textarea} label={i18n._("ai.questions.inputLabel")} value={draft} onChange={setDraft}
        placeholder={i18n._("ai.questions.placeholder")} rows={3} maxLength={2000} width="100%"
        status={[...draft].length > 2000 ? { type: "error", message: i18n._("ai.questions.tooLong") } : undefined} />
      {error ? <Banner status="warning" title={i18n._("ai.questions.failed")} description={i18n._(`ai.questions.error.${error}`)} /> : null}
      <div className="article-question-actions">
        <Text type="supporting" color="secondary">{pending ? i18n._("ai.questions.responding") : i18n._("ai.questions.sessionHint")}</Text>
        <div className="article-question-submit">
          {pending ? <Button label={i18n._("ai.questions.stop")} variant="secondary" onClick={stop} />
            : <Button label={i18n._(error && draft.trim() === retryQuestion ? "ai.questions.retry" : "ai.questions.send")} type="submit" variant="primary" isDisabled={!draft.trim() || [...draft.trim()].length > 2000} />}
        </div>
      </div>
      {provider ? <Text type="supporting" color="secondary">{i18n._("ai.reader.generatedBy", { provider })}</Text> : null}
    </form>
  </section>
}
