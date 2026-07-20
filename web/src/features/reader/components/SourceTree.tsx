import { Banner } from "@astryxdesign/core/Banner"
import { EmptyState } from "@astryxdesign/core/EmptyState"
import { Skeleton } from "@astryxdesign/core/Skeleton"
import type { TreeListDensity } from "@astryxdesign/core/TreeList"
import { TextInput } from "@astryxdesign/core/TextInput"
import { useLingui } from "@lingui/react"
import { useState, type Ref } from "react"

import { CategoryList } from "../categories/CategoryList"
import type { ReaderSource, ReaderState } from "../model/types"
import { refreshPresentation } from "../refresh/refreshPresentation"
import { RefreshStatusSummary } from "../refresh/RefreshStatusSummary"
import { SourceToolbar } from "./ReaderToolbar"

interface SourceTreeProps {
  state: ReaderState
  onSelect: (source: ReaderSource) => void
  onAdd: () => void
  onManage: () => void
  onManageSubscription?: () => void
  onPreferences: () => void
  onTransferSubscriptions: () => void
  onRefresh: (subscriptionId: string) => Promise<void>
  onLogout: () => Promise<void>
  manageButtonRef?: Ref<HTMLButtonElement>
  manageSubscriptionButtonRef?: Ref<HTMLButtonElement>
  preferencesButtonRef?: Ref<HTMLButtonElement>
  density: TreeListDensity
}

export function SourceTree({
  state,
  onSelect,
  onAdd,
  onManage,
  onManageSubscription,
  onPreferences,
  onTransferSubscriptions,
  onRefresh,
  onLogout,
  manageButtonRef,
  manageSubscriptionButtonRef,
  preferencesButtonRef,
  density,
}: SourceTreeProps) {
  const { i18n } = useLingui()
  const [sourceQuery, setSourceQuery] = useState("")
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
    <div className="reader-source-tree" aria-busy={state.paneStatus.subscriptions === "loading"}>
      <SourceToolbar
        onAdd={onAdd}
        onManage={onManage}
        onManageSubscription={
          selectedSubscription && onManageSubscription
            ? onManageSubscription
            : undefined
        }
        onPreferences={onPreferences}
        onTransferSubscriptions={onTransferSubscriptions}
        onLogout={onLogout}
        manageButtonRef={manageButtonRef}
        manageSubscriptionButtonRef={manageSubscriptionButtonRef}
        preferencesButtonRef={preferencesButtonRef}
        refresh={selectedSubscription ? {
          label: i18n._("reader.refreshFeed", { title: selectedSubscription.title }),
          onRefresh: () => onRefresh(selectedSubscription.subscriptionId),
          isDisabled: selectedRefresh?.isPending ?? false,
        } : undefined}
      />
      {selectedSubscription ? (
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
      <CategoryList
        state={state}
        onSelect={onSelect}
        density={density}
        query={sourceQuery}
      />
      {state.paneStatus.subscriptions === "error" ? (
        <Banner
          container="section"
          status="error"
          title={i18n._("reader.subscriptionsError")}
          description={state.errors.subscriptions ?? i18n._("reader.genericError")}
        />
      ) : state.paneStatus.subscriptions === "loading" ? (
        <div className="reader-skeletons" role="status" aria-label={i18n._("reader.loadingSubscriptions")}>
          {[0, 1, 2].map((index) => <Skeleton key={index} height={44} radius={2} index={index} />)}
        </div>
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
