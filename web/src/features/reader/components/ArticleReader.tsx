import { Banner } from "@astryxdesign/core/Banner"
import { EmptyState } from "@astryxdesign/core/EmptyState"
import { Skeleton } from "@astryxdesign/core/Skeleton"
import { useLingui } from "@lingui/react"

import type { ReaderState } from "../model/types"
import { ArticleToolbar } from "./ReaderToolbar"

interface ArticleReaderProps {
  state: ReaderState
  showBack: boolean
  onBack: () => void
  onToggleRead: (entryId: string) => Promise<void>
  onToggleStar: (entryId: string) => Promise<void>
}

export function ArticleReader(props: ArticleReaderProps) {
  const { i18n } = useLingui()
  const detail = props.state.selectedEntryId ? props.state.detailsById[props.state.selectedEntryId] : undefined
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
        showBack={props.showBack}
        isRead={detail.isRead}
        isStarred={detail.isStarred}
        canonicalUrl={detail.canonicalUrl}
        onBack={props.onBack}
        onToggleRead={() => props.onToggleRead(detail.entryId)}
        onToggleStar={() => props.onToggleStar(detail.entryId)}
      />
      <article className="reader-article">
        <p className="reader-article-kicker">{detail.feedTitle}</p>
        <h1>{detail.title ?? i18n._("reader.untitled")}</h1>
        <div className="reader-article-body" dangerouslySetInnerHTML={{ __html: detail.contentHtml }} />
      </article>
    </div>
  )
}
