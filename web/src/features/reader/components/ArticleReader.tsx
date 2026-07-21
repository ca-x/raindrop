import { Banner } from "@astryxdesign/core/Banner"
import { Icon } from "@astryxdesign/core/Icon"
import { EmptyState } from "@astryxdesign/core/EmptyState"
import { Skeleton } from "@astryxdesign/core/Skeleton"
import { useLingui } from "@lingui/react"
import { useEffect, useLayoutEffect, useMemo, useRef } from "react"

import { AiReaderSidecar } from "../../ai/reader/AiReaderSidecar"
import { useEntryAiController } from "../../ai/model/useEntryAiController"
import type { ReaderState } from "../model/types"
import type { UserPreferencesLinkOpenMode } from "../../preferences/api/preferences.generated"
import type {
  UserFont,
  UserPreferencesReadingFontFamily,
} from "../../preferences/api/preferences.generated"
import { ArticleToolbar, ReadingFloatingToolbar } from "./ReaderToolbar"

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
  aiOperations?: { summary: boolean; translation: boolean }
  linkOpenMode?: UserPreferencesLinkOpenMode
  readingFontScale?: number
  readingFontFamily?: UserPreferencesReadingFontFamily
  readingCustomFontId?: string | null
  fonts?: UserFont[]
  isReadingPreferenceSaving?: boolean
  onReadingFontScaleChange?: (scale: number) => Promise<boolean>
  onReadingFontChange?: (
    family: UserPreferencesReadingFontFamily,
    customFontId: string | null,
  ) => Promise<boolean>
}

const ignoreUnauthenticated = () => {}

export function ArticleReader(props: ArticleReaderProps) {
  const { i18n } = useLingui()
  const linkOpenMode = props.linkOpenMode ?? "NEW_TAB"
  const readingFontScale = props.readingFontScale ?? 100
  const readingFontFamily = props.readingFontFamily ?? "SERIF"
  const onReadingFontScaleChange =
    props.onReadingFontScaleChange ?? (async () => true)
  const onReadingFontChange = props.onReadingFontChange ?? (async () => true)
  const articleRef = useRef<HTMLElement>(null)
  const bodyRef = useRef<HTMLDivElement>(null)
  const headingRef = useRef<HTMLHeadingElement>(null)
  const summaryButtonRef = useRef<HTMLButtonElement>(null)
  const translationButtonRef = useRef<HTMLButtonElement>(null)
  const activeAiTrigger = useRef<HTMLButtonElement | null>(null)
  const detail = props.state.selectedEntryId ? props.state.detailsById[props.state.selectedEntryId] : undefined
  const detailMatchesRoute = Boolean(detail && detail.entryId === props.routeEntryId)
  const canBindArticle = detailMatchesRoute && props.state.paneStatus.detail === "ready"
  const articleHtml = useMemo(
    () => detail
      ? prepareArticleHtml(
          detail.entryId,
          detail.contentHtml,
          detail.inertImages,
          linkOpenMode,
          i18n._("reader.imageUnavailable"),
        )
      : "",
    [detail?.contentHtml, detail?.entryId, detail?.inertImages, i18n.locale, linkOpenMode],
  )
  const articleMarkup = useMemo(() => ({ __html: articleHtml }), [articleHtml])
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
  useLayoutEffect(() => {
    if (!articleHtml || !bodyRef.current) return
    return enhanceArticleContent(bodyRef.current)
  })
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
        linkOpenMode={linkOpenMode}
        onToggleRead={() => props.onToggleRead(detail.entryId)}
        onToggleStar={() => props.onToggleStar(detail.entryId)}
        onOpenSummary={
          props.csrfToken && props.aiOperations?.summary
            ? () => {
                activeAiTrigger.current = summaryButtonRef.current
                aiController.open("summary")
              }
            : undefined
        }
        onOpenTranslation={
          props.csrfToken && props.aiOperations?.translation
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
      >
        <p className="reader-article-kicker">{detail.feedTitle}</p>
        <h1 ref={headingRef} tabIndex={-1}>{detail.title ?? i18n._("reader.untitled")}</h1>
        <div
          ref={bodyRef}
          className="reader-article-body"
          dangerouslySetInnerHTML={articleMarkup}
        />
        {safeHttpUrl(detail.canonicalUrl) ? (
          <a
            className="reader-open-original"
            href={detail.canonicalUrl ?? undefined}
            target={linkOpenMode === "NEW_TAB" ? "_blank" : undefined}
            rel={
              linkOpenMode === "NEW_TAB"
                ? "noopener noreferrer"
                : undefined
            }
          >
            <span>{i18n._("reader.openOriginal")}</span>
            <Icon icon="externalLink" />
          </a>
        ) : null}
      </article>
      <ReadingFloatingToolbar
        readingFontScale={readingFontScale}
        readingFontFamily={readingFontFamily}
        readingCustomFontId={props.readingCustomFontId ?? null}
        fonts={props.fonts ?? []}
        isSaving={props.isReadingPreferenceSaving ?? false}
        onScaleChange={onReadingFontScaleChange}
        onFontChange={onReadingFontChange}
      />
    </div>
  )
}

function prepareArticleHtml(
  entryId: string,
  contentHtml: string,
  inertImages: Array<{
    imageIndex: number
    sourceUrl: string
    alt: string | null
    width: number | null
    height: number | null
  }>,
  linkOpenMode: UserPreferencesLinkOpenMode,
  imageUnavailableLabel: string,
): string {
  const template = document.createElement("template")
  template.innerHTML = contentHtml
  const images = Array.from(template.content.querySelectorAll<HTMLImageElement>("img"))
  for (const metadata of inertImages) {
    const image =
      template.content.querySelector<HTMLImageElement>(
        `img[data-raindrop-inert-image="${metadata.imageIndex}"]`,
      ) ?? images[metadata.imageIndex]
    if (!image || !safeHttpUrl(metadata.sourceUrl)) continue
    image.setAttribute("loading", "lazy")
    image.setAttribute("decoding", "async")
    image.setAttribute("referrerpolicy", "no-referrer")
    image.dataset.raindropImageState = "loading"
    image.src = `/reader-assets/entries/${encodeURIComponent(entryId)}/images/${metadata.imageIndex}`
    const frame = document.createElement("span")
    frame.className = "reader-article-image-frame"
    frame.dataset.raindropImageState = "loading"
    frame.dataset.raindropImageLabel = imageUnavailableLabel
    frame.setAttribute("role", "group")
    if (metadata.width && metadata.height) {
      frame.style.setProperty("--reader-image-aspect", `${metadata.width} / ${metadata.height}`)
      frame.style.setProperty("--reader-image-width", `${metadata.width}px`)
    }
    image.before(frame)
    frame.append(image)
  }

  for (const anchor of template.content.querySelectorAll<HTMLAnchorElement>("a[href]")) {
    if (!safeHttpUrl(anchor.href)) continue
    if (linkOpenMode === "NEW_TAB") {
      anchor.target = "_blank"
      anchor.rel = mergeRel(anchor.rel, ["noopener", "noreferrer", "nofollow"])
    } else {
      anchor.removeAttribute("target")
    }
  }
  return template.innerHTML
}

function enhanceArticleContent(body: HTMLElement): () => void {
  const cleanups: Array<() => void> = []
  for (const image of body.querySelectorAll<HTMLImageElement>(
    "img[data-raindrop-image-state]",
  )) {
    const frame = image.closest<HTMLElement>(".reader-article-image-frame")
    const onLoad = () => {
      image.dataset.raindropImageState = "loaded"
      if (frame) frame.dataset.raindropImageState = "loaded"
    }
    const onError = () => {
      image.removeAttribute("src")
      image.dataset.raindropImageState = "error"
      image.hidden = true
      if (frame) {
        frame.dataset.raindropImageState = "error"
        frame.setAttribute("role", "img")
        frame.setAttribute(
          "aria-label",
          image.alt || frame.dataset.raindropImageLabel || "",
        )
      }
    }
    image.addEventListener("load", onLoad)
    image.addEventListener("error", onError)
    if (image.dataset.raindropImageState === "loading" && image.complete) {
      if (image.naturalWidth > 0) onLoad()
      else onError()
    }
    cleanups.push(() => {
      image.removeEventListener("load", onLoad)
      image.removeEventListener("error", onError)
    })
  }
  return () => cleanups.forEach((cleanup) => cleanup())
}

function safeHttpUrl(raw: string | null): string | null {
  if (!raw) return null
  try {
    const url = new URL(raw, window.location.href)
    return url.protocol === "http:" || url.protocol === "https:" ? url.href : null
  } catch {
    return null
  }
}

function mergeRel(current: string, required: string[]): string {
  return [...new Set([...current.split(/\s+/u).filter(Boolean), ...required])].join(" ")
}

function clampOffset(element: HTMLElement, offset: number): number {
  return Math.max(0, Math.min(offset, Math.max(0, element.scrollHeight - element.clientHeight)))
}
