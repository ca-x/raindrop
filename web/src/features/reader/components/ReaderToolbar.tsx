import { Button } from "@astryxdesign/core/Button"
import { Icon } from "@astryxdesign/core/Icon"
import { Kbd } from "@astryxdesign/core/Kbd"
import { MoreMenu } from "@astryxdesign/core/MoreMenu"
import { Stack } from "@astryxdesign/core/Stack"
import { Text } from "@astryxdesign/core/Text"
import { ToggleButton } from "@astryxdesign/core/ToggleButton"
import { Toolbar } from "@astryxdesign/core/Toolbar"
import { useLingui } from "@lingui/react"
import type { Ref } from "react"

import { BrandMark } from "../../../shared/brand/BrandMark"
import type { UserPreferencesLinkOpenMode } from "../../preferences/api/preferences.generated"

interface SourceToolbarProps {
  onAdd: () => void
  onManage: () => void
  onManageSubscription?: () => void
  onPreferences: () => void
  onTransferSubscriptions: () => void
  onLogout: () => Promise<void>
  manageButtonRef?: Ref<HTMLButtonElement>
  manageSubscriptionButtonRef?: Ref<HTMLButtonElement>
  preferencesButtonRef?: Ref<HTMLButtonElement>
  refresh?: { label: string; onRefresh: () => Promise<void>; isDisabled: boolean }
}

export function SourceToolbar({
  onAdd,
  onManage,
  onManageSubscription,
  onPreferences,
  onTransferSubscriptions,
  onLogout,
  manageButtonRef,
  manageSubscriptionButtonRef,
  preferencesButtonRef,
  refresh,
}: SourceToolbarProps) {
  const { i18n } = useLingui()
  return (
    <Toolbar
      label={i18n._("reader.sources.actions")}
      size="lg"
      dividers={["bottom"]}
      startContent={<BrandMark size="sm" />}
      endContent={
        <>
          <Button
            label={i18n._("reader.addSubscription")}
            icon={<PlusIcon />}
            isIconOnly
            tooltip={i18n._("reader.addSubscription")}
            onClick={onAdd}
            variant="ghost"
          />
          {refresh ? (
            <Button
              label={refresh.label}
              icon={<RefreshIcon />}
              isIconOnly
              tooltip={refresh.label}
              clickAction={refresh.onRefresh}
              isDisabled={refresh.isDisabled}
              variant="ghost"
            />
          ) : null}
          {onManageSubscription ? (
            <Button
              ref={manageSubscriptionButtonRef}
              label={i18n._("reader.manageFeed")}
              icon={<FeedSettingsIcon />}
              isIconOnly
              tooltip={i18n._("reader.manageFeed")}
              onClick={onManageSubscription}
              variant="ghost"
            />
          ) : null}
          <Button
            ref={manageButtonRef}
            label={i18n._("reader.manageCategories")}
            icon={<Icon icon="wrench" />}
            isIconOnly
            tooltip={i18n._("reader.manageCategories")}
            onClick={onManage}
            variant="ghost"
          />
          <MoreMenu
            ref={preferencesButtonRef}
            label={i18n._("common.menu")}
            size="lg"
            items={[
              { label: i18n._("preferences.open"), onClick: onPreferences },
              { label: i18n._("opml.open"), onClick: onTransferSubscriptions },
              { type: "divider" },
              { label: i18n._("common.logout"), onClick: () => void onLogout() },
            ]}
          />
        </>
      }
    />
  )
}

interface CompactArticleNavigationProps {
  onOpenSources: () => void
  onBack: () => void
}

export function CompactArticleNavigation({ onOpenSources, onBack }: CompactArticleNavigationProps) {
  const { i18n } = useLingui()
  return (
    <div className="reader-compact-navigation">
      <Toolbar
        label={i18n._("reader.articleNavigation")}
        size="lg"
        dividers={["bottom"]}
        startContent={(
          <>
            <Button
              label={i18n._("reader.openSources")}
              icon={<Icon icon="menu" />}
              isIconOnly
              tooltip={i18n._("reader.openSources")}
              onClick={onOpenSources}
              variant="ghost"
            />
            <Button
              label={i18n._("reader.backToQueue")}
              icon={<Icon icon="chevronLeft" />}
              onClick={onBack}
              variant="ghost"
            />
          </>
        )}
      />
    </div>
  )
}

interface ArticleToolbarProps {
  isRead: boolean
  isStarred: boolean
  canonicalUrl: string | null
  linkOpenMode: UserPreferencesLinkOpenMode
  readingFontScale: number
  isReadingPreferenceSaving: boolean
  onReadingFontScaleChange: (scale: number) => Promise<boolean>
  onToggleRead: () => Promise<void>
  onToggleStar: () => Promise<void>
  onOpenSummary?: () => void
  onOpenTranslation?: () => void
  summaryButtonRef?: Ref<HTMLButtonElement>
  translationButtonRef?: Ref<HTMLButtonElement>
}

export function ArticleToolbar(props: ArticleToolbarProps) {
  const { i18n } = useLingui()
  const updateScale = (scale: number) => {
    void props.onReadingFontScaleChange(Math.max(85, Math.min(130, scale)))
  }
  return (
    <Toolbar
      className="reader-article-toolbar"
      label={i18n._("reader.articleActions")}
      size="lg"
      dividers={["bottom"]}
      endContent={
        <>
          {props.onOpenSummary ? (
            <Button
              ref={props.summaryButtonRef}
              label={i18n._("ai.reader.summaryAction")}
              onClick={props.onOpenSummary}
              variant="secondary"
            />
          ) : null}
          {props.onOpenTranslation ? (
            <Button
              ref={props.translationButtonRef}
              label={i18n._("ai.reader.translationAction")}
              onClick={props.onOpenTranslation}
              variant="secondary"
            />
          ) : null}
          <span
            className="reader-reading-controls"
            role="group"
            aria-label={i18n._("reader.readingSizeControls")}
          >
            <Button
              label={i18n._("reader.decreaseReadingSize")}
              icon={<span aria-hidden="true">A−</span>}
              isIconOnly
              tooltip={i18n._("reader.decreaseReadingSize")}
              onClick={() => updateScale(props.readingFontScale - 5)}
              isDisabled={props.isReadingPreferenceSaving || props.readingFontScale <= 85}
              variant="ghost"
            />
            <Button
              label={i18n._("reader.resetReadingSize", { scale: props.readingFontScale })}
              tooltip={i18n._("reader.resetReadingSize", { scale: props.readingFontScale })}
              onClick={() => updateScale(100)}
              isDisabled={
                props.isReadingPreferenceSaving || props.readingFontScale === 100
              }
              variant="ghost"
            >
              {props.readingFontScale}%
            </Button>
            <Button
              label={i18n._("reader.increaseReadingSize")}
              icon={<span aria-hidden="true">A＋</span>}
              isIconOnly
              tooltip={i18n._("reader.increaseReadingSize")}
              onClick={() => updateScale(props.readingFontScale + 5)}
              isDisabled={props.isReadingPreferenceSaving || props.readingFontScale >= 130}
              variant="ghost"
            />
          </span>
          <span className="reader-toolbar-shortcut">
            <ToggleButton
              label={i18n._(props.isRead ? "reader.markUnread" : "reader.markRead")}
              icon={<Icon icon="checkDouble" />}
              isIconOnly
              isPressed={props.isRead}
              pressedChangeAction={props.onToggleRead}
              aria-keyshortcuts="M"
            />
            <span className="reader-shortcut-label">{i18n._("reader.readStateShortcut")}</span>
            <Kbd keys="m" />
          </span>
          <span className="reader-toolbar-shortcut">
            <ToggleButton
              label={i18n._(props.isStarred ? "reader.unstarEntry" : "reader.starEntry")}
              icon={<span aria-hidden="true">☆</span>}
              pressedIcon={<span aria-hidden="true">★</span>}
              isIconOnly
              isPressed={props.isStarred}
              pressedChangeAction={props.onToggleStar}
              aria-keyshortcuts="S"
            />
            <span className="reader-shortcut-label">{i18n._("reader.starStateShortcut")}</span>
            <Kbd keys="s" />
          </span>
          {props.canonicalUrl ? (
            <Button
              label={i18n._("reader.openOriginal")}
              icon={<Icon icon="externalLink" />}
              isIconOnly
              tooltip={i18n._("reader.openOriginal")}
              href={props.canonicalUrl}
              target={props.linkOpenMode === "NEW_TAB" ? "_blank" : undefined}
              rel={
                props.linkOpenMode === "NEW_TAB" ? "noopener noreferrer" : undefined
              }
              variant="ghost"
            />
          ) : null}
        </>
      }
    />
  )
}

function RefreshIcon() {
  return (
    <svg
      aria-hidden="true"
      viewBox="0 0 24 24"
      width="1em"
      height="1em"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M20 11a8 8 0 1 0-2.34 5.66" />
      <path d="M20 4v7h-7" />
    </svg>
  )
}

function PlusIcon() {
  return (
    <svg
      aria-hidden="true"
      viewBox="0 0 24 24"
      width="1em"
      height="1em"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
    >
      <path d="M12 5v14" />
      <path d="M5 12h14" />
    </svg>
  )
}

function FeedSettingsIcon() {
  return (
    <svg
      aria-hidden="true"
      viewBox="0 0 24 24"
      width="1em"
      height="1em"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <circle cx="6" cy="18" r="1" fill="currentColor" stroke="none" />
      <path d="M5 11a8 8 0 0 1 8 8" />
      <path d="M5 5a14 14 0 0 1 14 14" />
      <path d="M17 4v4" />
      <path d="M15 6h4" />
    </svg>
  )
}
