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

interface SourceToolbarProps {
  onAdd: () => void
  onManage: () => void
  onPreferences: () => void
  onTransferSubscriptions: () => void
  onLogout: () => Promise<void>
  manageButtonRef?: Ref<HTMLButtonElement>
  preferencesButtonRef?: Ref<HTMLButtonElement>
  refresh?: { label: string; onRefresh: () => Promise<void>; isDisabled: boolean }
}

export function SourceToolbar({
  onAdd,
  onManage,
  onPreferences,
  onTransferSubscriptions,
  onLogout,
  manageButtonRef,
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
          {refresh ? (
            <Button
              label={refresh.label}
              icon={<Icon icon="arrowDown" />}
              isIconOnly
              tooltip={refresh.label}
              clickAction={refresh.onRefresh}
              isDisabled={refresh.isDisabled}
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
          <Button
            label={i18n._("reader.addSubscription")}
            icon={<span aria-hidden="true">＋</span>}
            isIconOnly
            tooltip={i18n._("reader.addSubscription")}
            onClick={onAdd}
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
  onToggleRead: () => Promise<void>
  onToggleStar: () => Promise<void>
  onOpenSummary?: () => void
  onOpenTranslation?: () => void
  summaryButtonRef?: Ref<HTMLButtonElement>
  translationButtonRef?: Ref<HTMLButtonElement>
}

export function ArticleToolbar(props: ArticleToolbarProps) {
  const { i18n } = useLingui()
  return (
    <Toolbar
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
              target="_blank"
              rel="noopener noreferrer"
              variant="ghost"
            />
          ) : null}
        </>
      }
    />
  )
}
