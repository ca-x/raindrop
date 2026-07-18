import { Button } from "@astryxdesign/core/Button"
import { Icon } from "@astryxdesign/core/Icon"
import { Kbd } from "@astryxdesign/core/Kbd"
import { MoreMenu } from "@astryxdesign/core/MoreMenu"
import { Toolbar } from "@astryxdesign/core/Toolbar"
import { useLingui } from "@lingui/react"

export type MarkReadAvailability = "hidden" | "disabled" | "enabled"

interface QueueToolbarProps {
  showMenu: boolean
  isCompact: boolean
  markReadAvailability: MarkReadAvailability
  isMarkingRead: boolean
  onOpenSources: () => void
  onReload: () => Promise<void>
  onNextUnreadSource: () => Promise<void>
  onPreviousUnreadSource: () => Promise<void>
  onRequestMarkRead: () => void
}

export function QueueToolbar(props: QueueToolbarProps) {
  const { i18n } = useLingui()
  const markReadItem = props.markReadAvailability === "hidden"
    ? []
    : [
        { type: "divider" as const },
        {
          label:
            props.markReadAvailability === "disabled"
              ? i18n._("reader.markAllReadUnavailable")
              : i18n._("reader.markAllRead"),
          onClick: props.onRequestMarkRead,
          isDisabled:
            props.markReadAvailability === "disabled" || props.isMarkingRead,
          icon: <Icon icon="checkDouble" />,
        },
      ]
  const toolbar = (
    <Toolbar
      label={i18n._("reader.queue.actions")}
      size="lg"
      dividers={props.showMenu ? ["bottom"] : undefined}
      startContent={props.showMenu ? (
        <Button
          label={i18n._("reader.openSources")}
          icon={<Icon icon="menu" />}
          isIconOnly
          tooltip={i18n._("reader.openSources")}
          onClick={props.onOpenSources}
          variant="ghost"
        />
      ) : undefined}
      centerContent={<strong>{i18n._("reader.queue")}</strong>}
      endContent={
        <>
          <MoreMenu
            label={i18n._("reader.queueMenu")}
            size="lg"
            items={[
              {
                label: i18n._("reader.nextUnreadSource"),
                onClick: () => void props.onNextUnreadSource(),
                icon: <Icon icon="chevronRight" />,
              },
              {
                label: i18n._("reader.previousUnreadSource"),
                onClick: () => void props.onPreviousUnreadSource(),
                icon: <Icon icon="chevronLeft" />,
              },
              ...markReadItem,
            ]}
          />
          <Button
            label={i18n._("reader.reloadStored")}
            icon={<Icon icon="arrowsUpDown" />}
            isIconOnly
            tooltip={i18n._("reader.reloadStored")}
            clickAction={props.onReload}
            variant="ghost"
          />
        </>
      }
    />
  )
  const content = (
    <>
      {toolbar}
      {!props.showMenu ? (
        <div
          className="reader-queue-shortcuts"
          aria-label={i18n._("reader.queueShortcuts")}
        >
          <span className="reader-shortcut-label">
            {i18n._("reader.openEntryShortcut")}
          </span>
          <Kbd keys="j" /><Kbd keys="k" />
          <span className="reader-shortcut-label">
            {i18n._("reader.moveCursorShortcut")}
          </span>
          <Kbd keys="n" /><Kbd keys="p" />
          <span className="reader-shortcut-label">
            {i18n._("reader.unreadSourceShortcut")}
          </span>
          <Kbd keys="shift+j" /><Kbd keys="shift+k" />
        </div>
      ) : null}
    </>
  )
  return props.isCompact ? (
    <div className="reader-compact-navigation">{content}</div>
  ) : content
}
