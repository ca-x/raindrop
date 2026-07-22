import { Banner } from "@astryxdesign/core/Banner"
import { Icon } from "@astryxdesign/core/Icon"
import { EmptyState } from "@astryxdesign/core/EmptyState"
import { Skeleton } from "@astryxdesign/core/Skeleton"
import { useLingui } from "@lingui/react"
import { useEffect, useLayoutEffect, useMemo, useRef } from "react"

import { AiReaderSidecar } from "../../ai/reader/AiReaderSidecar"
import { useEntryAiController } from "../../ai/model/useEntryAiController"
import type { TranslationConfig } from "../../translation/api/translation.generated"
import { useEntryTranslationController } from "../../translation/model/useEntryTranslationController"
import type { TranslationSettingsController } from "../../translation/model/useTranslationSettingsController"
import { ArticleSelectionPopover } from "../../translation/reader/ArticleSelectionPopover"
import { TranslationReaderControls } from "../../translation/reader/TranslationReaderControls"
import type { ReaderState } from "../model/types"
import type { UserPreferencesLinkOpenMode } from "../../preferences/api/preferences.generated"
import type {
  UserFont,
  UserPreferencesReadingColorScheme,
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
  summaryEnabled?: boolean
  translationConfig?: TranslationConfig | null
  translationSettingsController?: TranslationSettingsController
  linkOpenMode?: UserPreferencesLinkOpenMode
  readingFontScale?: number
  readingFontFamily?: UserPreferencesReadingFontFamily
  readingCustomFontId?: string | null
  readingColorScheme?: UserPreferencesReadingColorScheme
  fonts?: UserFont[]
  isReadingPreferenceSaving?: boolean
  onReadingFontScaleChange?: (scale: number) => Promise<boolean>
  onReadingFontChange?: (
    family: UserPreferencesReadingFontFamily,
    customFontId: string | null,
  ) => Promise<boolean>
  onReadingColorSchemeChange?: (
    colorScheme: UserPreferencesReadingColorScheme,
  ) => Promise<boolean>
}

const ignoreUnauthenticated = () => {}

export function ArticleReader(props: ArticleReaderProps) {
  const { i18n } = useLingui()
  const linkOpenMode = props.linkOpenMode ?? "NEW_TAB"
  const readingFontScale = props.readingFontScale ?? 100
  const readingFontFamily = props.readingFontFamily ?? "SERIF"
  const readingColorScheme = props.readingColorScheme ?? "AUTO"
  const onReadingFontScaleChange =
    props.onReadingFontScaleChange ?? (async () => true)
  const onReadingFontChange = props.onReadingFontChange ?? (async () => true)
  const onReadingColorSchemeChange =
    props.onReadingColorSchemeChange ?? (async () => true)
  const articleRef = useRef<HTMLElement>(null)
  const bodyRef = useRef<HTMLDivElement>(null)
  const headingRef = useRef<HTMLHeadingElement>(null)
  const summaryButtonRef = useRef<HTMLButtonElement>(null)
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
  const translationController = useEntryTranslationController(
    detailMatchesRoute ? detail?.entryId ?? null : null,
    props.csrfToken ?? "",
    props.onUnauthenticated ?? ignoreUnauthenticated,
    props.translationConfig?.engine === "DEEPLX" &&
      props.translationConfig.deepLx.isProgressive,
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
  useLayoutEffect(() => {
    if (!bodyRef.current || !translationController.result || !props.translationConfig) {
      return
    }
    return applyImmersiveTranslation(
      bodyRef.current,
      translationController.result.segments,
      props.translationConfig.displayMode,
      props.translationConfig.defaultTargetLocale,
    )
  }, [
    articleHtml,
    props.translationConfig?.defaultTargetLocale,
    props.translationConfig?.displayMode,
    translationController.result,
  ])
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
          props.csrfToken && props.summaryEnabled
            ? () => {
                activeAiTrigger.current = summaryButtonRef.current
                aiController.open("summary")
              }
            : undefined
        }
        summaryButtonRef={summaryButtonRef}
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
      {props.translationConfig?.isEnabled && props.translationSettingsController ? (
        <TranslationReaderControls
          controller={translationController}
          config={props.translationConfig}
          onDisplayModeChange={props.translationSettingsController.saveDisplayMode}
        />
      ) : null}
      <article
        ref={articleRef}
        className="reader-article"
        lang={i18n.locale}
      >
        <p className="reader-article-kicker">{detail.feedTitle}</p>
        <div
          className="reader-translation-title-pair"
          data-translation-mode={
            translationController.result && props.translationConfig
              ? props.translationConfig.displayMode
              : undefined
          }
        >
          <h1 className="reader-translation-original" ref={headingRef} tabIndex={-1}>
            {detail.title ?? i18n._("reader.untitled")}
          </h1>
          {translationController.result ? (
            <div className="reader-translation-title" lang={props.translationConfig?.defaultTargetLocale}>
              {translationController.result.title}
            </div>
          ) : null}
        </div>
        <ArticleSelectionPopover
          controller={translationController}
          isEnabled={props.translationConfig?.isEnabled ?? false}
        >
          <div
            ref={bodyRef}
            className="reader-article-body"
            dangerouslySetInnerHTML={articleMarkup}
          />
        </ArticleSelectionPopover>
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
        readingColorScheme={readingColorScheme}
        fonts={props.fonts ?? []}
        isSaving={props.isReadingPreferenceSaving ?? false}
        onScaleChange={onReadingFontScaleChange}
        onFontChange={onReadingFontChange}
        onColorSchemeChange={onReadingColorSchemeChange}
      />
    </div>
  )
}

const TRANSLATION_BLOCK_SELECTOR = [
  "p",
  "li",
  "blockquote",
  "pre",
  "h1",
  "h2",
  "h3",
  "h4",
  "h5",
  "h6",
  "figcaption",
  "td",
  "th",
  "div",
].join(",")

function applyImmersiveTranslation(
  body: HTMLElement,
  segments: Array<{ index: number; originalText: string; translatedText: string }>,
  mode: TranslationConfig["displayMode"],
  targetLocale: string,
): () => void {
  const candidates = Array.from(
    body.querySelectorAll<HTMLElement>(TRANSLATION_BLOCK_SELECTOR),
  ).filter((element) => {
    if (!normalizeTranslationText(element.textContent ?? "")) return false
    return !Array.from(
      element.querySelectorAll<HTMLElement>(TRANSLATION_BLOCK_SELECTOR),
    ).some(
      (child) =>
        child !== element && normalizeTranslationText(child.textContent ?? ""),
    )
  })
  const used = new Set<HTMLElement>()
  let cursor = 0
  body.dataset.translationMode = mode
  for (const segment of segments) {
    const original = normalizeTranslationText(segment.originalText)
    let matched: HTMLElement | undefined
    for (let index = cursor; index < candidates.length; index += 1) {
      const candidate = candidates[index]
      if (!candidate || used.has(candidate)) continue
      if (normalizeTranslationText(candidate.textContent ?? "") === original) {
        matched = candidate
        cursor = index + 1
        break
      }
    }
    if (!matched) {
      matched = candidates.find(
        (candidate) =>
          !used.has(candidate) &&
          normalizeTranslationText(candidate.textContent ?? "") === original,
      )
    }
    if (!matched) continue
    used.add(matched)
    const originalContent = document.createElement("span")
    originalContent.className = "reader-translation-original"
    while (matched.firstChild) originalContent.append(matched.firstChild)
    const translation = document.createElement("span")
    translation.className = "reader-translation-segment"
    translation.lang = targetLocale
    translation.textContent = segment.translatedText
    matched.classList.add("reader-translation-pair")
    matched.dataset.translationIndex = String(segment.index)
    matched.append(originalContent, translation)
  }
  return () => {
    delete body.dataset.translationMode
    for (const pair of body.querySelectorAll<HTMLElement>(".reader-translation-pair")) {
      const original = pair.querySelector<HTMLElement>(":scope > .reader-translation-original")
      const translation = pair.querySelector<HTMLElement>(":scope > .reader-translation-segment")
      translation?.remove()
      if (original) {
        while (original.firstChild) pair.insertBefore(original.firstChild, original)
        original.remove()
      }
      pair.classList.remove("reader-translation-pair")
      delete pair.dataset.translationIndex
    }
  }
}

function normalizeTranslationText(value: string): string {
  return value.replace(/\s+/gu, " ").trim()
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
