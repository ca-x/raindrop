import { Banner } from "@astryxdesign/core/Banner"
import { Button } from "@astryxdesign/core/Button"
import { EmptyState } from "@astryxdesign/core/EmptyState"
import { Skeleton } from "@astryxdesign/core/Skeleton"
import type { TreeListDensity } from "@astryxdesign/core/TreeList"
import { TextInput } from "@astryxdesign/core/TextInput"
import { useLingui } from "@lingui/react"
import { useState, type Ref } from "react"

import { CategoryList } from "../categories/CategoryList"
import {
  isAuthoritativeSubscription,
  type ReaderSource,
  type ReaderState,
} from "../model/types"
import { refreshPresentation } from "../refresh/refreshPresentation"
import { RefreshStatusSummary } from "../refresh/RefreshStatusSummary"
import { SourceToolbar } from "./ReaderToolbar"
import { sourceTreeDensityMetrics } from "./sourceDensity"

interface SourceTreeProps {
  state: ReaderState
  onSelect: (source: ReaderSource) => void
  onRequestMarkRead?: (feedId: string, title: string) => void
  onMoveSubscription: (
    subscriptionId: string,
    categoryId: string | null,
  ) => Promise<boolean>
  isMarkingRead?: boolean
  onManage: () => void
  onEditSubscription: () => void
  onPreferences: () => void
  onRefresh: (subscriptionId: string) => Promise<void>
  onRetrySubscriptions?: () => Promise<boolean>
  onLogout: () => Promise<void>
  manageButtonRef?: Ref<HTMLButtonElement>
  editSubscriptionButtonRef?: Ref<HTMLButtonElement>
  preferencesButtonRef?: Ref<HTMLButtonElement>
  density: TreeListDensity
}

export function SourceTree({
  state,
  onSelect,
  onRequestMarkRead,
  onMoveSubscription,
  isMarkingRead = false,
  onManage,
  onEditSubscription,
  onPreferences,
  onRefresh,
  onRetrySubscriptions = async () => false,
  onLogout,
  manageButtonRef,
  editSubscriptionButtonRef,
  preferencesButtonRef,
  density,
}: SourceTreeProps) {
  const { i18n } = useLingui()
  const [sourceQuery, setSourceQuery] = useState("")
  const [showAllSources, setShowAllSources] = useState(true)
  const selectedFeedId = state.selectedSource.kind === "feed" ? state.selectedSource.feedId : null
  const selectedSubscription = selectedFeedId
    ? state.subscriptionOrder
      .map((id) => state.subscriptionsById[id])
      .find((subscription) => subscription.feedId === selectedFeedId)
    : undefined
  const selectedRefresh = selectedSubscription
    ? refreshPresentation(selectedSubscription.refresh)
    : null
  return (
    <div
      className="reader-source-tree"
      aria-busy={state.requestActivity.subscriptions}
      data-request-active={state.requestActivity.subscriptions ? "true" : undefined}
    >
      <div className="reader-pane-progress" aria-hidden="true" />
      <span className="reader-visually-hidden" role="status" aria-live="polite">
        {state.requestActivity.subscriptions
          ? i18n._("reader.syncingSubscriptions")
          : ""}
      </span>
      <SourceToolbar
        onManage={onManage}
        onEditSubscription={
          state.subscriptionsAuthoritative &&
          selectedSubscription &&
          isAuthoritativeSubscription(selectedSubscription)
            ? onEditSubscription
            : undefined
        }
        onPreferences={onPreferences}
        onLogout={onLogout}
        manageButtonRef={manageButtonRef}
        editSubscriptionButtonRef={editSubscriptionButtonRef}
        preferencesButtonRef={preferencesButtonRef}
        refresh={selectedSubscription ? {
          label: i18n._("reader.refreshFeed", { title: selectedSubscription.title }),
          onRefresh: () => onRefresh(selectedSubscription.subscriptionId),
          isDisabled: selectedRefresh?.isPending ?? false,
        } : undefined}
      />
      {selectedSubscription &&
      selectedRefresh &&
      selectedRefresh.kind !== "idle" ? (
        <RefreshStatusSummary refresh={selectedSubscription.refresh} />
      ) : null}
      {state.subscriptionOrder.length > 6 ? (
        <div className="reader-source-search">
          <TextInput
            label={i18n._("reader.searchSources")}
            isLabelHidden
            placeholder={i18n._("reader.searchSourcesPlaceholder")}
            value={sourceQuery}
            onChange={setSourceQuery}
            hasClear
            size="sm"
            width="100%"
          />
        </div>
      ) : null}
      {state.subscriptionOrder.length > 0 ? (
        <div className="reader-source-view-options">
          <Button
            className="reader-source-visibility-toggle"
            label={i18n._(
              showAllSources
                ? "reader.showUnreadSources"
                : "reader.showAllSources",
            )}
            variant="ghost"
            size="sm"
            isDisabled={sourceQuery.trim().length > 0}
            onClick={() => setShowAllSources((current) => !current)}
          />
        </div>
      ) : null}
      {state.paneStatus.subscriptions === "loading" &&
      state.subscriptionOrder.length === 0 ? (
        <div className="reader-skeletons" role="status" aria-label={i18n._("reader.loadingSubscriptions")}>
          {[0, 1, 2, 3, 4].map((index) => (
            <Skeleton
              key={index}
              height={sourceTreeDensityMetrics[density].rowBlockSize}
              radius={2}
              index={index}
            />
          ))}
        </div>
      ) : (
        <CategoryList
          state={state}
          onSelect={onSelect}
          onRequestMarkRead={onRequestMarkRead}
          onMoveSubscription={onMoveSubscription}
          isMarkingRead={isMarkingRead}
          density={density}
          query={sourceQuery}
          showAllSources={showAllSources || sourceQuery.trim().length > 0}
        />
      )}
      {state.errors.subscriptions ? (
        <Banner
          container="section"
          status="error"
          title={i18n._("reader.subscriptionsError")}
          description={state.errors.subscriptions ?? i18n._("reader.genericError")}
          endContent={(
            <Button
              label={i18n._("common.retry")}
              variant="ghost"
              size="sm"
              clickAction={async () => { await onRetrySubscriptions() }}
            />
          )}
        />
      ) : null}
      {state.subscriptionOrder.length === 0 &&
      (state.paneStatus.subscriptions === "idle" || state.paneStatus.subscriptions === "ready") ? (
        <EmptyState
          isCompact
          title={i18n._("reader.noSubscriptions")}
          description={i18n._("reader.noSubscriptionsDescription")}
        />
      ) : null}
    </div>
  )
}
