import { Button } from "@astryxdesign/core/Button"
import { Icon } from "@astryxdesign/core/Icon"
import { Kbd } from "@astryxdesign/core/Kbd"
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
  onLogout: () => Promise<void>
  manageButtonRef?: Ref<HTMLButtonElement>
  refresh?: { label: string; onRefresh: () => Promise<void> }
}

export function SourceToolbar({
  onAdd,
  onManage,
  onLogout,
  manageButtonRef,
  refresh,
}: SourceToolbarProps) {
  const { i18n } = useLingui()
  return (
    <Toolbar
      label={i18n._("reader.sources.actions")}
      size="lg"
      dividers={["bottom"]}
      startContent={(
        <Stack direction="horizontal" gap={2} align="center">
          <BrandMark size="sm" decorative />
          <Text type="label">Raindrop</Text>
        </Stack>
      )}
      endContent={
        <>
          {refresh ? (
            <Button
              label={refresh.label}
              icon={<Icon icon="arrowDown" />}
              isIconOnly
              tooltip={refresh.label}
              clickAction={refresh.onRefresh}
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
          <Button label={i18n._("reader.addSubscription")} onClick={onAdd} variant="ghost" />
          <Button label={i18n._("common.logout")} clickAction={onLogout} variant="ghost" />
        </>
      }
    />
  )
}

interface QueueToolbarProps {
  showMenu: boolean
  isCompact: boolean
  onOpenSources: () => void
  onReload: () => Promise<void>
}

export function QueueToolbar({ showMenu, isCompact, onOpenSources, onReload }: QueueToolbarProps) {
  const { i18n } = useLingui()
  const toolbar = (
    <Toolbar
      label={i18n._("reader.queue.actions")}
      size="lg"
      dividers={["bottom"]}
      startContent={showMenu ? (
        <Button
          label={i18n._("reader.openSources")}
          icon={<Icon icon="menu" />}
          isIconOnly
          tooltip={i18n._("reader.openSources")}
          onClick={onOpenSources}
          variant="ghost"
        />
      ) : undefined}
      centerContent={<strong>{i18n._("reader.queue")}</strong>}
      endContent={
        <>
          {!isCompact ? (
            <span className="reader-queue-shortcuts" aria-label={i18n._("reader.queueShortcuts")}>
              <span className="reader-shortcut-label">{i18n._("reader.openEntryShortcut")}</span>
              <Kbd keys="j" /><Kbd keys="k" />
              <span className="reader-shortcut-label">{i18n._("reader.moveCursorShortcut")}</span>
              <Kbd keys="n" /><Kbd keys="p" />
            </span>
          ) : null}
          <Button
            label={i18n._("reader.reloadStored")}
            icon={<Icon icon="arrowsUpDown" />}
            isIconOnly
            tooltip={i18n._("reader.reloadStored")}
            clickAction={onReload}
            variant="ghost"
          />
        </>
      }
    />
  )
  return isCompact ? <div className="reader-compact-navigation">{toolbar}</div> : toolbar
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
