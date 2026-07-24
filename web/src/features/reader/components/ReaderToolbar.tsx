import { Button } from "@astryxdesign/core/Button"
import { Icon } from "@astryxdesign/core/Icon"
import { Kbd } from "@astryxdesign/core/Kbd"
import { MoreMenu } from "@astryxdesign/core/MoreMenu"
import { Popover } from "@astryxdesign/core/Popover"
import { Stack } from "@astryxdesign/core/Stack"
import { Text } from "@astryxdesign/core/Text"
import { ToggleButton } from "@astryxdesign/core/ToggleButton"
import { Toolbar } from "@astryxdesign/core/Toolbar"
import { useLingui } from "@lingui/react"
import { useState, type Ref } from "react"

import { BrandMark } from "../../../shared/brand/BrandMark"
import type {
  UserFont,
  UserPreferencesLinkOpenMode,
  UserPreferencesReadingColorScheme,
  UserPreferencesReadingFontFamily,
} from "../../preferences/api/preferences.generated"
import { StarIcon } from "./StarIcon"

interface SourceToolbarProps {
  onManage: () => void
  onEditSubscription?: () => void
  onPreferences: () => void
  onLogout: () => Promise<void>
  manageButtonRef?: Ref<HTMLButtonElement>
  editSubscriptionButtonRef?: Ref<HTMLButtonElement>
  preferencesButtonRef?: Ref<HTMLButtonElement>
  refresh?: { label: string; onRefresh: () => Promise<void>; isDisabled: boolean }
}

export function SourceToolbar({
  onManage,
  onEditSubscription,
  onPreferences,
  onLogout,
  manageButtonRef,
  editSubscriptionButtonRef,
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
            ref={manageButtonRef}
            label={i18n._("reader.manageSubscriptions")}
            icon={<PlusIcon />}
            isIconOnly
            tooltip={i18n._("reader.manageSubscriptions")}
            onClick={onManage}
            variant="ghost"
          />
          {refresh ? (
            <Button
              label={refresh.label}
              icon={<RefreshIcon />}
              isIconOnly
              tooltip={refresh.label}
              onClick={() => void refresh.onRefresh()}
              isDisabled={refresh.isDisabled}
              variant="ghost"
            />
          ) : null}
          {onEditSubscription ? (
            <Button
              ref={editSubscriptionButtonRef}
              label={i18n._("reader.editSubscription")}
              icon={<Icon icon="wrench" />}
              isIconOnly
              tooltip={i18n._("reader.editSubscription")}
              onClick={onEditSubscription}
              variant="ghost"
            />
          ) : null}
          <MoreMenu
            ref={preferencesButtonRef}
            label={i18n._("common.menu")}
            size="lg"
            items={[
              { label: i18n._("preferences.open"), onClick: onPreferences },
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
  onToggleRead: () => Promise<void>
  onToggleStar: () => Promise<void>
  onOpenSummary?: () => void
  summaryButtonRef?: Ref<HTMLButtonElement>
}

export function ArticleToolbar(props: ArticleToolbarProps) {
  const { i18n } = useLingui()
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
              icon={<StarIcon />}
              pressedIcon={<StarIcon isFilled />}
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

interface ReadingFloatingToolbarProps {
  readingFontScale: number
  readingFontFamily: UserPreferencesReadingFontFamily
  readingCustomFontId: string | null
  readingColorScheme: UserPreferencesReadingColorScheme
  fonts: UserFont[]
  isSaving: boolean
  onScaleChange: (scale: number) => Promise<boolean>
  onFontChange: (
    family: UserPreferencesReadingFontFamily,
    customFontId: string | null,
  ) => Promise<boolean>
  onColorSchemeChange: (
    colorScheme: UserPreferencesReadingColorScheme,
  ) => Promise<boolean>
}

export function ReadingFloatingToolbar(props: ReadingFloatingToolbarProps) {
  const { i18n } = useLingui()
  const [isExpanded, setIsExpanded] = useState(false)
  const [isFontOpen, setIsFontOpen] = useState(false)
  const [isColorOpen, setIsColorOpen] = useState(false)
  const updateScale = (scale: number) => {
    void props.onScaleChange(Math.max(85, Math.min(130, scale)))
  }
  const value = props.readingCustomFontId
    ? `custom:${props.readingCustomFontId}`
    : props.readingFontFamily
  const selectFont = (
    family: UserPreferencesReadingFontFamily,
    customFontId: string | null,
  ) => {
    void props.onFontChange(family, customFontId).then((saved) => {
      if (saved) setIsFontOpen(false)
    })
  }
  const selectColorScheme = (
    colorScheme: UserPreferencesReadingColorScheme,
  ) => {
    void props.onColorSchemeChange(colorScheme).then((saved) => {
      if (saved) setIsColorOpen(false)
    })
  }
  const isDockExpanded = isExpanded || isFontOpen || isColorOpen
  return (
    <div
      className="reader-reading-dock"
      data-expanded={isDockExpanded ? "true" : undefined}
      tabIndex={0}
      role="group"
      aria-label={i18n._("reader.openReadingControls")}
    >
      <Button
        className="reader-reading-dock-trigger"
        label={i18n._(isExpanded ? "reader.closeReadingControls" : "reader.openReadingControls")}
        icon={(
          <span className="reader-reading-controls-icon" aria-hidden="true">
            <span />
            <span />
          </span>
        )}
        isIconOnly
        tooltip={i18n._(isExpanded ? "reader.closeReadingControls" : "reader.openReadingControls")}
        onClick={() => setIsExpanded((current) => !current)}
        variant="secondary"
      />
      <div
        className="reader-reading-float"
        role="toolbar"
        aria-label={i18n._("reader.readingDisplayControls")}
      >
        <Popover
          isOpen={isFontOpen}
          onOpenChange={setIsFontOpen}
          placement="above"
          alignment="start"
          width="min(260px, calc(100vw - 24px))"
          label={i18n._("reader.readingFontMenu")}
          closeButtonLabel={i18n._("common.close")}
          content={(
            <div className="reader-reading-popover-list" role="group" aria-label={i18n._("preferences.readingFont")}>
              <ReadingOption
                label={i18n._("preferences.fontSerif")}
                isSelected={value === "SERIF"}
                isDisabled={props.isSaving}
                onClick={() => selectFont("SERIF", null)}
              />
              <ReadingOption
                label={i18n._("preferences.fontSans")}
                isSelected={value === "SANS"}
                isDisabled={props.isSaving}
                onClick={() => selectFont("SANS", null)}
              />
              {props.fonts.length > 0 ? (
                <div className="reader-reading-popover-divider" aria-hidden="true" />
              ) : null}
              {props.fonts.map((font) => (
                <ReadingOption
                  key={font.fontId}
                  label={font.displayName}
                  isSelected={value === `custom:${font.fontId}`}
                  isDisabled={props.isSaving}
                  onClick={() => selectFont(props.readingFontFamily, font.fontId)}
                />
              ))}
            </div>
          )}
        >
          {(trigger) => (
            <Button
              ref={trigger.ref}
              label={i18n._("reader.readingFontMenu")}
              icon={<span className="reader-reading-font-icon" aria-hidden="true">Aa</span>}
              isIconOnly
              tooltip={i18n._("reader.readingFontMenu")}
              isDisabled={props.isSaving}
              onClick={trigger.onClick}
              aria-haspopup={trigger["aria-haspopup"]}
              aria-expanded={trigger["aria-expanded"]}
              aria-controls={trigger["aria-controls"]}
              variant="ghost"
            />
          )}
        </Popover>
        <span className="reader-floating-divider" aria-hidden="true" />
        <Button
          label={i18n._("reader.decreaseReadingSize")}
          icon={<span aria-hidden="true">A−</span>}
          isIconOnly
          tooltip={i18n._("reader.decreaseReadingSize")}
          onClick={() => updateScale(props.readingFontScale - 5)}
          isDisabled={props.isSaving || props.readingFontScale <= 85}
          variant="ghost"
        />
        <Button
          label={i18n._("reader.resetReadingSize", { scale: props.readingFontScale })}
          tooltip={i18n._("reader.resetReadingSize", { scale: props.readingFontScale })}
          onClick={() => updateScale(100)}
          isDisabled={props.isSaving || props.readingFontScale === 100}
          variant="ghost"
        >
          <span className="reader-reading-scale-value">{props.readingFontScale}%</span>
        </Button>
        <Button
          label={i18n._("reader.increaseReadingSize")}
          icon={<span aria-hidden="true">A＋</span>}
          isIconOnly
          tooltip={i18n._("reader.increaseReadingSize")}
          onClick={() => updateScale(props.readingFontScale + 5)}
          isDisabled={props.isSaving || props.readingFontScale >= 130}
          variant="ghost"
        />
        <span className="reader-floating-divider" aria-hidden="true" />
        <Popover
          isOpen={isColorOpen}
          onOpenChange={setIsColorOpen}
          placement="above"
          alignment="end"
          width="min(240px, calc(100vw - 24px))"
          label={i18n._("reader.readingThemeMenu")}
          closeButtonLabel={i18n._("common.close")}
          content={(
            <div className="reader-reading-popover-list" role="group" aria-label={i18n._("preferences.readingColor")}>
              {([
                ["AUTO", "preferences.colorAuto"],
                ["PAPER", "preferences.colorPaper"],
                ["SEPIA", "preferences.colorSepia"],
                ["GRAY", "preferences.colorGray"],
              ] as const).map(([colorScheme, label]) => (
                <ReadingOption
                  key={colorScheme}
                  label={i18n._(label)}
                  isSelected={props.readingColorScheme === colorScheme}
                  isDisabled={props.isSaving}
                  onClick={() => selectColorScheme(colorScheme)}
                  swatch={colorScheme}
                />
              ))}
            </div>
          )}
        >
          {(trigger) => (
            <Button
              ref={trigger.ref}
              label={i18n._("reader.readingThemeMenu")}
              icon={<span className="reader-reading-theme-icon" aria-hidden="true" />}
              isIconOnly
              tooltip={i18n._("reader.readingThemeMenu")}
              isDisabled={props.isSaving}
              onClick={trigger.onClick}
              aria-haspopup={trigger["aria-haspopup"]}
              aria-expanded={trigger["aria-expanded"]}
              aria-controls={trigger["aria-controls"]}
              variant="ghost"
            />
          )}
        </Popover>
      </div>
    </div>
  )
}

interface ReadingOptionProps {
  label: string
  isSelected: boolean
  isDisabled: boolean
  onClick: () => void
  swatch?: UserPreferencesReadingColorScheme
}

function ReadingOption(props: ReadingOptionProps) {
  return (
    <button
      type="button"
      className="reader-reading-popover-option"
      aria-pressed={props.isSelected}
      disabled={props.isDisabled}
      onClick={props.onClick}
    >
      {props.swatch ? (
        <span
          className="reader-reading-theme-swatch"
          data-reading-theme={props.swatch.toLowerCase()}
          aria-hidden="true"
        />
      ) : null}
      <span>{props.label}</span>
      <span className="reader-reading-option-check" aria-hidden="true">
        {props.isSelected ? "✓" : ""}
      </span>
    </button>
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
