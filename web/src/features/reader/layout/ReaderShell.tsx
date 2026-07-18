import { AppShell } from "@astryxdesign/core/AppShell"
import { Banner } from "@astryxdesign/core/Banner"
import { MobileNav } from "@astryxdesign/core/MobileNav"
import { useResizable } from "@astryxdesign/core/Resizable"
import { useLingui } from "@lingui/react"
import { useRef, useState } from "react"

import type { ViewportMode } from "../../../shared/responsive/useViewportMode"
import { PreferencesDialog } from "../../preferences/components/PreferencesDialog"
import { toAstryxDensity } from "../../preferences/model/preferenceTypes"
import type { PreferencesController } from "../../preferences/model/usePreferencesController"
import { ArticleReader } from "../components/ArticleReader"
import { EntryQueue } from "../components/EntryQueue"
import { MarkReadDialog } from "../components/MarkReadDialog"
import { MutationFeedback } from "../components/MutationFeedback"
import { SourceTree } from "../components/SourceTree"
import { SubscriptionDialog } from "../components/SubscriptionDialog"
import { ReaderCategoryDialog } from "../categories/ReaderCategoryDialog"
import { useReaderHotkeys } from "../keyboard/useReaderHotkeys"
import { sourceKey, type ReaderSource } from "../model/types"
import { selectedSourceLabel } from "../model/sourcePresentation"
import type { ReaderController } from "../model/useReaderController"
import { pathForEntry, type ReaderRouteMatch } from "../routes/readerRoute"
import { ReaderWorkspacePanels } from "./ReaderWorkspacePanels"

interface ReaderShellProps {
  controller: ReaderController
  preferencesController: PreferencesController
  route: ReaderRouteMatch
  isSourceReady: boolean
  username: string
  viewportMode: ViewportMode
  onLogout: () => Promise<void>
  sessionError?: string | null
  onSelectSource: (source: ReaderSource) => void
  onSelectEntry: (entryId: string) => void
  onOpenEntryFromHotkey: (entryId: string) => void
  onNextUnreadSource: () => Promise<void>
  onPreviousUnreadSource: () => Promise<void>
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
  const [isPreferencesOpen, setIsPreferencesOpen] = useState(false)
  const [isMarkReadOpen, setIsMarkReadOpen] = useState(false)
  const mobileNavRef = useRef<HTMLDialogElement>(null)
  const categoryButtonRef = useRef<HTMLButtonElement>(null)
  const preferencesButtonRef = useRef<HTMLButtonElement>(null)
  const reopenSourcesAfterCategory = useRef(false)
  const reopenSourcesAfterPreferences = useRef(false)
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
    isDisabled:
      isNavOpen ||
      isAddOpen ||
      isCategoryOpen ||
      isPreferencesOpen ||
      isMarkReadOpen ||
      !props.isSourceReady,
    isUnread: (entryId) => {
      const entry = props.controller.state.entriesById[entryId] ?? props.controller.state.detailsById[entryId]
      return entry ? !entry.isRead : false
    },
    onCursorChange: props.onCursorChange,
    onOpenEntry: props.onOpenEntryFromHotkey,
    onToggleRead: props.controller.toggleRead,
    onToggleStar: props.controller.toggleStar,
    onNextUnreadSource: props.onNextUnreadSource,
    onPreviousUnreadSource: props.onPreviousUnreadSource,
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
      onPreferences={() => {
        reopenSourcesAfterPreferences.current = props.viewportMode !== "wide"
        if (reopenSourcesAfterPreferences.current) {
          mobileNavRef.current?.close()
          setIsNavOpen(false)
        }
        setIsPreferencesOpen(true)
      }}
      onRefresh={props.controller.refreshSubscription}
      onLogout={async () => {
        mobileNavRef.current?.close()
        setIsNavOpen(false)
        await props.onLogout()
      }}
      manageButtonRef={categoryButtonRef}
      preferencesButtonRef={preferencesButtonRef}
      density={toAstryxDensity(
        props.preferencesController.preferences.layoutDensity,
      )}
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
      onSearchFeed={props.controller.searchFeed}
      onNextUnreadSource={props.onNextUnreadSource}
      onPreviousUnreadSource={props.onPreviousUnreadSource}
      onRequestMarkRead={() => setIsMarkReadOpen(true)}
      isMarkingRead={props.controller.isMarkingRead}
      onMergePending={props.controller.mergePendingEntries}
      onMergedEntryFocus={props.onCursorChange}
      density={toAstryxDensity(
        props.preferencesController.preferences.layoutDensity,
      )}
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
      <ReaderWorkspacePanels
        viewportMode={props.viewportMode}
        hasEntry={Boolean(props.route.entryId)}
        queueStatus={props.controller.state.paneStatus.queue}
        detailStatus={props.controller.state.paneStatus.detail}
        sourceTree={sourceTree}
        queuePane={queuePane}
        articlePane={articlePane}
        sourcesResizable={sources.props}
        queueResizable={queue.props}
        onOpenSources={() => setIsNavOpen(true)}
        onBack={props.onBack}
      />
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
      <PreferencesDialog
        isOpen={isPreferencesOpen}
        preferences={props.preferencesController.preferences}
        isSaving={props.preferencesController.isSaving}
        error={props.preferencesController.error}
        onClearError={props.preferencesController.clearError}
        onSave={props.preferencesController.save}
        onOpenChange={(open) => {
          setIsPreferencesOpen(open)
          if (open) return
          if (reopenSourcesAfterPreferences.current) {
            reopenSourcesAfterPreferences.current = false
            setIsNavOpen(true)
            requestAnimationFrame(() =>
              requestAnimationFrame(() => preferencesButtonRef.current?.focus()),
            )
            return
          }
          requestAnimationFrame(() => preferencesButtonRef.current?.focus())
        }}
      />
      <MarkReadDialog
        isOpen={isMarkReadOpen}
        sourceLabel={selectedSourceLabel(props.controller.state, (id) => i18n._(id))}
        isLoading={props.controller.isMarkingRead}
        onOpenChange={setIsMarkReadOpen}
        onConfirm={props.controller.markCurrentSourceRead}
      />
      <MutationFeedback
        error={props.controller.state.errors.mutation}
        isDialogOpen={
          isAddOpen || isCategoryOpen || isPreferencesOpen || isMarkReadOpen
        }
        onClear={props.controller.clearMutationError}
      />
    </AppShell>
  )
}
