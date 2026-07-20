import { Banner } from "@astryxdesign/core/Banner"
import { EmptyState } from "@astryxdesign/core/EmptyState"
import { Skeleton } from "@astryxdesign/core/Skeleton"
import { useLingui } from "@lingui/react"
import { useEffect, useLayoutEffect, useRef } from "react"

import { AiReaderSidecar } from "../../ai/reader/AiReaderSidecar"
import { useEntryAiController } from "../../ai/model/useEntryAiController"
import type { ReaderState } from "../model/types"
import { ArticleToolbar } from "./ReaderToolbar"

interface ArticleReaderProps {
  state: ReaderState
  entryRoute: string | null
  routeEntryId: string | null
  savedScrollOffset: number
  shouldFocusArticle: boolean
  onRecordScroll: (route: string, offset: number) => void
  onToggleRead: (entryId: string) => Promise<void>
  onToggleStar: (entryId: string) => Promise<void>
  csrfToken?: string
  onUnauthenticated?: () => void
  onOpenAiSettings?: () => void
}

const ignoreUnauthenticated = () => {}

export function ArticleReader(props: ArticleReaderProps) {
  const { i18n } = useLingui()
  const articleRef = useRef<HTMLElement>(null)
  const headingRef = useRef<HTMLHeadingElement>(null)
  const summaryButtonRef = useRef<HTMLButtonElement>(null)
  const translationButtonRef = useRef<HTMLButtonElement>(null)
  const activeAiTrigger = useRef<HTMLButtonElement | null>(null)
  const detail = props.state.selectedEntryId ? props.state.detailsById[props.state.selectedEntryId] : undefined
  const detailMatchesRoute = Boolean(detail && detail.entryId === props.routeEntryId)
  const canBindArticle = detailMatchesRoute && props.state.paneStatus.detail === "ready"
  const aiController = useEntryAiController(
    detailMatchesRoute ? detail?.entryId ?? null : null,
    props.csrfToken ?? "",
    props.onUnauthenticated ?? ignoreUnauthenticated,
  )
  useLayoutEffect(() => {
    const node = articleRef.current
    const entryRoute = props.entryRoute
    if (!node || !canBindArticle || !entryRoute) return
    node.scrollTop = clampOffset(node, props.savedScrollOffset)
    return () => props.onRecordScroll(entryRoute, node.scrollTop)
  }, [canBindArticle, detail?.entryId, props.entryRoute])
  useEffect(() => {
    if (canBindArticle && props.shouldFocusArticle) headingRef.current?.focus({ preventScroll: true })
  }, [canBindArticle, detail?.entryId, props.shouldFocusArticle])
  if (props.state.selectedEntryId && props.state.paneStatus.detail === "error") {
    return (
      <Banner
        container="section"
        status="error"
        title={i18n._("reader.articleError")}
        description={props.state.errors.detail ?? i18n._("reader.genericError")}
      />
    )
  }
  if (props.state.selectedEntryId && props.state.paneStatus.detail === "loading") {
    return (
      <div className="reader-article-loading" role="status" aria-label={i18n._("reader.loadingArticle")}>
        <Skeleton height={36} width="70%" radius={2} />
        <Skeleton height={18} width="40%" radius={2} index={1} />
        <Skeleton height={240} width="100%" radius={2} index={2} />
      </div>
    )
  }
  if (!detail) {
    return (
      <div className="reader-article-empty">
        <EmptyState
          title={i18n._("reader.selectArticle")}
          description={i18n._("reader.selectArticleDescription")}
        />
      </div>
    )
  }
  return (
    <div className="reader-article-plane">
      <ArticleToolbar
        isRead={detail.isRead}
        isStarred={detail.isStarred}
        canonicalUrl={detail.canonicalUrl}
        onToggleRead={() => props.onToggleRead(detail.entryId)}
        onToggleStar={() => props.onToggleStar(detail.entryId)}
        onOpenSummary={
          props.csrfToken
            ? () => {
                activeAiTrigger.current = summaryButtonRef.current
                aiController.open("summary")
              }
            : undefined
        }
        onOpenTranslation={
          props.csrfToken
            ? () => {
                activeAiTrigger.current = translationButtonRef.current
                aiController.open("translation")
              }
            : undefined
        }
        summaryButtonRef={summaryButtonRef}
        translationButtonRef={translationButtonRef}
      />
      {aiController.openTab ? (
        <AiReaderSidecar
          controller={aiController}
          onOpenSettings={props.onOpenAiSettings}
          onClose={() => {
            aiController.close()
            requestAnimationFrame(() => activeAiTrigger.current?.focus())
          }}
        />
      ) : null}
      <article
        ref={articleRef}
        className="reader-article"
        lang={i18n.locale}
        onScroll={(event) => {
          if (canBindArticle && props.entryRoute) {
            props.onRecordScroll(props.entryRoute, event.currentTarget.scrollTop)
          }
        }}
      >
        <p className="reader-article-kicker">{detail.feedTitle}</p>
        <h1 ref={headingRef} tabIndex={-1}>{detail.title ?? i18n._("reader.untitled")}</h1>
        <div className="reader-article-body" dangerouslySetInnerHTML={{ __html: detail.contentHtml }} />
      </article>
    </div>
  )
}

function clampOffset(element: HTMLElement, offset: number): number {
  return Math.max(0, Math.min(offset, Math.max(0, element.scrollHeight - element.clientHeight)))
}
