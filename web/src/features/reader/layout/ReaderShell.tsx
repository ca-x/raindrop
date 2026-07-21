import { AppShell } from "@astryxdesign/core/AppShell"
import { Banner } from "@astryxdesign/core/Banner"
import { MobileNav } from "@astryxdesign/core/MobileNav"
import { useResizable } from "@astryxdesign/core/Resizable"
import { useLingui } from "@lingui/react"
import { useRef, useState } from "react"

import type { ViewportMode } from "../../../shared/responsive/useViewportMode"
import type { AiSettingsController } from "../../ai/model/useAiSettingsController"
import { PreferencesDialog } from "../../preferences/components/PreferencesDialog"
import type { PreferencesTab } from "../../preferences/components/PreferencesDialog"
import { toAstryxDensity } from "../../preferences/model/preferenceTypes"
import type { PreferencesController } from "../../preferences/model/usePreferencesController"
import type { ProfileController } from "../../profile/model/useProfileController"
import type { TranslationSettingsController } from "../../translation/model/useTranslationSettingsController"
import { ArticleReader } from "../components/ArticleReader"
import { EntryQueue } from "../components/EntryQueue"
import { MarkReadDialog } from "../components/MarkReadDialog"
import { MutationFeedback } from "../components/MutationFeedback"
import { SourceTree } from "../components/SourceTree"
import { SubscriptionEditDialog } from "../components/SubscriptionEditDialog"
import { SubscriptionManagementDialog } from "../components/SubscriptionManagementDialog"
import { useReaderHotkeys } from "../keyboard/useReaderHotkeys"
import { sourceKey, type ReaderSource } from "../model/types"
import { selectedSourceLabel } from "../model/sourcePresentation"
import type { ReaderController } from "../model/useReaderController"
import { pathForEntry, type ReaderRouteMatch } from "../routes/readerRoute"
import { ReaderWorkspacePanels } from "./ReaderWorkspacePanels"

interface ReaderShellProps {
  controller: ReaderController
  preferencesController: PreferencesController
  profileController?: ProfileController
  aiSettingsController?: AiSettingsController
  translationController?: TranslationSettingsController
  route: ReaderRouteMatch
  isSourceReady: boolean
  username: string
  email?: string | null
  viewportMode: ViewportMode
  onLogout: () => Promise<void>
  onUnauthenticated?: () => void
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
  const [isManagementOpen, setIsManagementOpen] = useState(false)
  const [isSubscriptionEditOpen, setIsSubscriptionEditOpen] = useState(false)
  const [isPreferencesOpen, setIsPreferencesOpen] = useState(false)
  const [preferencesInitialTab, setPreferencesInitialTab] =
    useState<PreferencesTab>("personal")
  const [isMarkReadOpen, setIsMarkReadOpen] = useState(false)
  const mobileNavRef = useRef<HTMLDialogElement>(null)
  const managementButtonRef = useRef<HTMLButtonElement>(null)
  const editSubscriptionButtonRef = useRef<HTMLButtonElement>(null)
  const preferencesButtonRef = useRef<HTMLButtonElement>(null)
  const reopenSourcesAfterManagement = useRef(false)
  const reopenSourcesAfterSubscriptionEdit = useRef(false)
  const reopenSourcesAfterPreferences = useRef(false)
  const sources = useResizable({ defaultSize: 240, minSizePx: 200, maxSizePx: 340, autoSaveId: "reader-sources" })
  const queue = useResizable({ defaultSize: 380, minSizePx: 300, maxSizePx: 560, autoSaveId: "reader-queue" })
  const accountLabel = props.profileController?.profile.displayName || props.username
  const queueEntryIds = props.isSourceReady
    ? props.controller.state.queueBySourceKey[sourceKey(props.controller.state.selectedSource)] ?? []
    : []
  const selectedSource = props.controller.state.selectedSource
  const selectedSubscription =
    selectedSource.kind === "feed"
      ? props.controller.state.subscriptionOrder
          .map((id) => props.controller.state.subscriptionsById[id])
          .find(
            (subscription) => subscription.feedId === selectedSource.feedId,
          )
      : undefined
  const entryRoute = props.route.entryId ? pathForEntry(props.route.sourcePath, props.route.entryId) : null
  const aiConfig = props.aiSettingsController?.configEnvelope?.config
  const summaryEnabled = Boolean(aiConfig?.isEnabled && aiConfig.summary.enabled)
  const translationConfig = props.translationController?.config ?? null
  useReaderHotkeys({
    queueEntryIds,
    cursorEntryId: props.cursorEntryId,
    openEntryId: props.route.entryId,
    isDisabled:
      isNavOpen ||
      isManagementOpen ||
      isSubscriptionEditOpen ||
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
      onManage={() => {
        reopenSourcesAfterManagement.current = props.viewportMode !== "wide"
        if (reopenSourcesAfterManagement.current) {
          mobileNavRef.current?.close()
          setIsNavOpen(false)
        }
        setIsManagementOpen(true)
      }}
      onEditSubscription={() => {
        if (!selectedSubscription) return
        reopenSourcesAfterSubscriptionEdit.current = props.viewportMode !== "wide"
        if (reopenSourcesAfterSubscriptionEdit.current) {
          mobileNavRef.current?.close()
          setIsNavOpen(false)
        }
        setIsSubscriptionEditOpen(true)
      }}
      onPreferences={() => {
        setPreferencesInitialTab("personal")
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
      manageButtonRef={managementButtonRef}
      editSubscriptionButtonRef={editSubscriptionButtonRef}
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
      csrfToken={props.aiSettingsController?.csrfToken}
      summaryEnabled={summaryEnabled}
      translationConfig={translationConfig}
      translationSettingsController={props.translationController}
      linkOpenMode={props.preferencesController.preferences.linkOpenMode}
      readingFontScale={props.preferencesController.preferences.readingFontScale}
      readingFontFamily={props.preferencesController.preferences.readingFontFamily}
      readingCustomFontId={props.preferencesController.preferences.readingCustomFontId}
      readingColorScheme={props.preferencesController.preferences.readingColorScheme}
      fonts={props.preferencesController.fonts}
      isReadingPreferenceSaving={props.preferencesController.isSaving}
      onReadingFontScaleChange={(readingFontScale) =>
        props.preferencesController.save({
          ...props.preferencesController.preferences,
          readingFontScale,
        })
      }
      onReadingFontChange={(readingFontFamily, readingCustomFontId) =>
        props.preferencesController.save({
          ...props.preferencesController.preferences,
          readingFontFamily,
          readingCustomFontId,
        })
      }
      onReadingColorSchemeChange={(readingColorScheme) =>
        props.preferencesController.save({
          ...props.preferencesController.preferences,
          readingColorScheme,
        })
      }
      onUnauthenticated={props.onUnauthenticated}
      onOpenAiSettings={() => {
        reopenSourcesAfterPreferences.current = false
        setPreferencesInitialTab("plugins")
        setIsPreferencesOpen(true)
      }}
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
            header={`Raindrop · ${accountLabel}`}
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
      <SubscriptionManagementDialog
        isOpen={isManagementOpen}
        subscriptions={props.controller.state.subscriptionOrder.map(
          (id) => props.controller.state.subscriptionsById[id],
        )}
        categories={props.controller.state.categoryOrder.map(
          (id) => props.controller.state.categoriesById[id],
        )}
        mutationError={props.controller.state.errors.mutation}
        csrfToken={props.preferencesController.csrfToken}
        onClearError={props.controller.clearMutationError}
        onAdd={props.controller.addSubscription}
        onUpdate={props.controller.updateSubscription}
        onDelete={async (subscriptionId) => {
          const deleted = await props.controller.deleteSubscription(subscriptionId)
          if (deleted) props.onSelectSource({ kind: "smart", state: "UNREAD" })
          return deleted
        }}
        onCreateCategory={props.controller.createCategory}
        onUpdateCategory={(categoryId, title) =>
          props.controller.updateCategory(categoryId, { title })
        }
        onDeleteCategory={async (categoryId) => {
          const wasActive =
            props.controller.state.selectedSource.kind === "category" &&
            props.controller.state.selectedSource.categoryId === categoryId
          const deleted = await props.controller.deleteCategory(categoryId)
          if (deleted && wasActive) {
            props.onSelectSource({ kind: "smart", state: "UNREAD" })
          }
          return deleted
        }}
        onSubscriptionsChanged={props.controller.load}
        onOpenChange={(open) => {
          setIsManagementOpen(open)
          if (open) return
          if (reopenSourcesAfterManagement.current) {
            reopenSourcesAfterManagement.current = false
            setIsNavOpen(true)
            requestAnimationFrame(() =>
              requestAnimationFrame(() => managementButtonRef.current?.focus()),
            )
            return
          }
          requestAnimationFrame(() => managementButtonRef.current?.focus())
        }}
      />
      <SubscriptionEditDialog
        isOpen={isSubscriptionEditOpen}
        subscription={selectedSubscription}
        categories={props.controller.state.categoryOrder.map(
          (id) => props.controller.state.categoriesById[id],
        )}
        mutationError={props.controller.state.errors.mutation}
        linkOpenMode={props.preferencesController.preferences.linkOpenMode}
        onClearError={props.controller.clearMutationError}
        onUpdate={props.controller.updateSubscription}
        onDelete={async (subscriptionId) => {
          const deleted = await props.controller.deleteSubscription(subscriptionId)
          if (deleted) props.onSelectSource({ kind: "smart", state: "UNREAD" })
          return deleted
        }}
        onOpenChange={(open) => {
          setIsSubscriptionEditOpen(open)
          if (open) return
          if (reopenSourcesAfterSubscriptionEdit.current) {
            reopenSourcesAfterSubscriptionEdit.current = false
            setIsNavOpen(true)
            requestAnimationFrame(() =>
              requestAnimationFrame(() => editSubscriptionButtonRef.current?.focus()),
            )
            return
          }
          requestAnimationFrame(() => editSubscriptionButtonRef.current?.focus())
        }}
      />
      <PreferencesDialog
        isOpen={isPreferencesOpen}
        initialTab={preferencesInitialTab}
        profile={props.profileController?.profile ?? {
          userId: "00000000-0000-4000-8000-000000000000",
          username: props.username,
          displayName: null,
          email: props.email ?? null,
        }}
        preferences={props.preferencesController.preferences}
        fonts={props.preferencesController.fonts}
        fontLimits={props.preferencesController.fontLimits}
        isSaving={props.preferencesController.isSaving}
        isProfileSaving={props.profileController?.isSaving ?? false}
        isFontMutating={props.preferencesController.isFontMutating}
        error={props.preferencesController.error}
        profileError={props.profileController?.error ?? null}
        profileFieldErrors={props.profileController?.fieldErrors ?? {}}
        aiController={props.aiSettingsController}
        translationController={props.translationController}
        onClearError={props.preferencesController.clearError}
        onSave={props.preferencesController.save}
        onSaveProfile={props.profileController?.save ?? (async () => true)}
        onUploadFont={props.preferencesController.uploadFont}
        onDeleteFont={props.preferencesController.deleteFont}
        onClearProfileError={props.profileController?.clearError ?? (() => undefined)}
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
          isManagementOpen ||
          isSubscriptionEditOpen ||
          isPreferencesOpen ||
          isMarkReadOpen
        }
        onClear={props.controller.clearMutationError}
      />
    </AppShell>
  )
}
