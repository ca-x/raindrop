import { AppShell } from "@astryxdesign/core/AppShell"
import { Banner } from "@astryxdesign/core/Banner"
import { Layout, LayoutContent, LayoutPanel } from "@astryxdesign/core/Layout"
import { MobileNav } from "@astryxdesign/core/MobileNav"
import { ResizeHandle, useResizable } from "@astryxdesign/core/Resizable"
import { useLingui } from "@lingui/react"
import { useRef, useState } from "react"

import type { ViewportMode } from "../../../shared/responsive/useViewportMode"
import { ArticleReader } from "../components/ArticleReader"
import { EntryQueue } from "../components/EntryQueue"
import { CompactArticleNavigation } from "../components/ReaderToolbar"
import { MutationFeedback } from "../components/MutationFeedback"
import { SourceTree } from "../components/SourceTree"
import { SubscriptionDialog } from "../components/SubscriptionDialog"
import { ReaderCategoryDialog } from "../categories/ReaderCategoryDialog"
import { useReaderHotkeys } from "../keyboard/useReaderHotkeys"
import { sourceKey, type ReaderSource } from "../model/types"
import type { ReaderController } from "../model/useReaderController"
import { pathForEntry, type ReaderRouteMatch } from "../routes/readerRoute"

interface ReaderShellProps {
  controller: ReaderController
  route: ReaderRouteMatch
  isSourceReady: boolean
  username: string
  viewportMode: ViewportMode
  onLogout: () => Promise<void>
  sessionError?: string | null
  onSelectSource: (source: ReaderSource) => void
  onSelectEntry: (entryId: string) => void
  onOpenEntryFromHotkey: (entryId: string) => void
  cursorEntryId: string | null
  cursorFocusNonce: number
  onCursorChange: (entryId: string) => void
  onBack: () => void
}

export function ReaderShell(props: ReaderShellProps) {
  const { i18n } = useLingui()
  const [isNavOpen, setIsNavOpen] = useState(false)
  const [isAddOpen, setIsAddOpen] = useState(false)
  const [isCategoryOpen, setIsCategoryOpen] = useState(false)
  const mobileNavRef = useRef<HTMLDialogElement>(null)
  const categoryButtonRef = useRef<HTMLButtonElement>(null)
  const reopenSourcesAfterCategory = useRef(false)
  const sources = useResizable({ defaultSize: 240, minSizePx: 200, maxSizePx: 340, autoSaveId: "reader-sources" })
  const queue = useResizable({ defaultSize: 380, minSizePx: 300, maxSizePx: 560, autoSaveId: "reader-queue" })
  const queueEntryIds = props.isSourceReady
    ? props.controller.state.queueBySourceKey[sourceKey(props.controller.state.selectedSource)] ?? []
    : []
  const entryRoute = props.route.entryId ? pathForEntry(props.route.sourcePath, props.route.entryId) : null
  useReaderHotkeys({
    queueEntryIds,
    cursorEntryId: props.cursorEntryId,
    openEntryId: props.route.entryId,
    isDisabled: isNavOpen || isAddOpen || isCategoryOpen || !props.isSourceReady,
    isUnread: (entryId) => {
      const entry = props.controller.state.entriesById[entryId] ?? props.controller.state.detailsById[entryId]
      return entry ? !entry.isRead : false
    },
    onCursorChange: props.onCursorChange,
    onOpenEntry: props.onOpenEntryFromHotkey,
    onToggleRead: props.controller.toggleRead,
    onToggleStar: props.controller.toggleStar,
  })
  const sourceTree = (
    <SourceTree
      state={props.controller.state}
      onSelect={(source) => {
        setIsNavOpen(false)
        props.onSelectSource(source)
      }}
      onAdd={() => setIsAddOpen(true)}
      onManage={() => {
        reopenSourcesAfterCategory.current = props.viewportMode !== "wide"
        if (reopenSourcesAfterCategory.current) {
          mobileNavRef.current?.close()
          setIsNavOpen(false)
        }
        setIsCategoryOpen(true)
      }}
      onRefresh={props.controller.refreshSubscription}
      onLogout={async () => {
        mobileNavRef.current?.close()
        setIsNavOpen(false)
        await props.onLogout()
      }}
      manageButtonRef={categoryButtonRef}
    />
  )
  const queuePane = (
    <EntryQueue
      state={props.controller.state}
      showMenu={props.viewportMode !== "wide"}
      isCompact={props.viewportMode === "compact"}
      onOpenSources={() => setIsNavOpen(true)}
      onSelect={props.onSelectEntry}
      isRouteReady={props.isSourceReady}
      cursorEntryId={props.cursorEntryId}
      cursorFocusNonce={props.cursorFocusNonce}
      sourceRoute={props.route.sourcePath}
      savedScrollOffset={props.controller.state.scrollAnchorByRoute[props.route.sourcePath] ?? 0}
      onRecordScroll={props.controller.recordScrollAnchor}
      onReload={props.controller.reloadEntries}
      onMergePending={props.controller.mergePendingEntries}
      onMergedEntryFocus={props.onCursorChange}
    />
  )
  const articlePane = (
    <ArticleReader
      state={props.controller.state}
      entryRoute={entryRoute}
      routeEntryId={props.route.entryId}
      savedScrollOffset={entryRoute ? props.controller.state.scrollAnchorByRoute[entryRoute] ?? 0 : 0}
      shouldFocusArticle={props.viewportMode === "compact"}
      onRecordScroll={props.controller.recordScrollAnchor}
      onToggleRead={props.controller.toggleRead}
      onToggleStar={props.controller.toggleStar}
    />
  )

  return (
    <AppShell
      contentPadding={0}
      height="fill"
      variant="section"
      banner={props.sessionError ? (
        <Banner container="section" status="error" title={props.sessionError} />
      ) : undefined}
      mobileNav={
        props.viewportMode === "wide" ? false : (
          <MobileNav
            ref={mobileNavRef}
            isOpen={isNavOpen}
            onOpenChange={setIsNavOpen}
            label={i18n._("reader.sources")}
            header={`Raindrop · ${props.username}`}
            className="reader-mobile-nav"
          >
            {sourceTree}
          </MobileNav>
        )
      }
    >
      {renderWorkspace(props.viewportMode, Boolean(props.route.entryId))}
      <SubscriptionDialog
        isOpen={isAddOpen}
        mutationError={props.controller.state.errors.mutation}
        onOpenChange={setIsAddOpen}
        onClearError={props.controller.clearMutationError}
        onAdd={props.controller.addSubscription}
      />
      <ReaderCategoryDialog
        controller={props.controller}
        isOpen={isCategoryOpen}
        onOpenChange={(open) => {
          setIsCategoryOpen(open)
          if (open) return
          if (reopenSourcesAfterCategory.current) {
            reopenSourcesAfterCategory.current = false
            setIsNavOpen(true)
            requestAnimationFrame(() =>
              requestAnimationFrame(() => categoryButtonRef.current?.focus()),
            )
            return
          }
          requestAnimationFrame(() => categoryButtonRef.current?.focus())
        }}
        onSelectSource={props.onSelectSource}
      />
      <MutationFeedback
        error={props.controller.state.errors.mutation}
        isDialogOpen={isAddOpen || isCategoryOpen}
        onClear={props.controller.clearMutationError}
      />
    </AppShell>
  )

  function renderWorkspace(mode: ViewportMode, hasEntry: boolean) {
    if (mode === "compact") {
      return (
        <Layout height="fill" padding={0} content={
          <LayoutContent
            padding={0}
            role="region"
            label={hasEntry ? i18n._("reader.article") : i18n._("reader.queue")}
            aria-busy={hasEntry ? props.controller.state.paneStatus.detail === "loading" : props.controller.state.paneStatus.queue === "loading"}
          >
            {hasEntry ? (
              <div className="reader-compact-detail">
                <CompactArticleNavigation
                  onOpenSources={() => setIsNavOpen(true)}
                  onBack={props.onBack}
                />
                <div className="reader-compact-article-content">{articlePane}</div>
              </div>
            ) : queuePane}
          </LayoutContent>
        } />
      )
    }
    return (
      <Layout
        height="fill"
        padding={0}
        start={
          <>
            {mode === "wide" ? (
              <>
                <LayoutPanel padding={0} role="navigation" label={i18n._("reader.sources")} resizable={sources.props}>{sourceTree}</LayoutPanel>
                <ResizeHandle hasDivider label={i18n._("reader.resizeSources")} resizable={sources.props} />
              </>
            ) : null}
            <LayoutPanel
              padding={0}
              role="region"
              label={i18n._("reader.queue")}
              aria-busy={props.controller.state.paneStatus.queue === "loading"}
              resizable={mode === "wide" ? queue.props : undefined}
              width={380}
            >{queuePane}</LayoutPanel>
            {mode === "wide" ? <ResizeHandle hasDivider label={i18n._("reader.resizeQueue")} resizable={queue.props} /> : null}
          </>
        }
        content={
          <LayoutContent
            padding={0}
            role="complementary"
            label={i18n._("reader.article")}
            aria-busy={props.controller.state.paneStatus.detail === "loading"}
          >{articlePane}</LayoutContent>
        }
      />
    )
  }
}
