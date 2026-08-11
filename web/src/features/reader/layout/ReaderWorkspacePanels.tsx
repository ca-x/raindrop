import { Layout, LayoutContent, LayoutPanel } from "@astryxdesign/core/Layout"
import { ResizeHandle, type ResizableProps } from "@astryxdesign/core/Resizable"
import { useLingui } from "@lingui/react"
import type { ReactNode, Ref } from "react"

import type { ViewportMode } from "../../../shared/responsive/useViewportMode"
import { CompactArticleNavigation } from "../components/ReaderToolbar"
import type { PaneStatus } from "../model/types"

interface ReaderWorkspacePanelsProps {
  viewportMode: ViewportMode
  hasEntry: boolean
  queueStatus: PaneStatus
  detailStatus: PaneStatus
  sourceTree: ReactNode
  queuePane: ReactNode
  articlePane: ReactNode
  sourcesResizable: ResizableProps
  queueResizable: ResizableProps
  onOpenSources: () => void
  onBack: () => void
  isImmersive: boolean
  keyboardTransition: boolean
  openImmersivePanel: "sources" | "queue" | null
  suppressedImmersivePanel: "sources" | "queue" | null
  onOpenImmersivePanel: (panel: "sources" | "queue") => void
  onWorkspacePointerMove: (point: { x: number; y: number }) => void
  articlePanelRef: Ref<HTMLDivElement>
}

export function ReaderWorkspacePanels(props: ReaderWorkspacePanelsProps) {
  const { i18n } = useLingui()
  if (props.viewportMode === "compact") {
    return (
      <Layout
        height="fill"
        padding={0}
        content={(
          <LayoutContent
            padding={0}
            role="region"
            label={props.hasEntry ? i18n._("reader.article") : i18n._("reader.queue")}
            aria-busy={
              props.hasEntry
                ? props.detailStatus === "loading"
                : props.queueStatus === "loading"
            }
          >
            {props.hasEntry ? (
              <div className="reader-compact-detail">
                <CompactArticleNavigation
                  onOpenSources={props.onOpenSources}
                  onBack={props.onBack}
                />
                <div className="reader-compact-article-content">
                  {props.articlePane}
                </div>
              </div>
            ) : props.queuePane}
          </LayoutContent>
        )}
      />
    )
  }

  return (
    <Layout
      className={[
        "reader-workspace-layout",
        props.isImmersive ? "reader-workspace-immersive" : "",
        props.openImmersivePanel ? "reader-workspace-panel-open" : "",
        props.keyboardTransition ? "reader-workspace-keyboard-transition" : "",
      ].filter(Boolean).join(" ")}
      height="fill"
      padding={0}
      onPointerMove={(event) => {
        props.onWorkspacePointerMove({ x: event.clientX, y: event.clientY })
      }}
      start={(
        <>
          <ImmersiveEdge
            side="sources"
            label={i18n._("reader.revealSources")}
            onOpen={
              props.isImmersive
                ? () => props.onOpenImmersivePanel("sources")
                : props.onOpenSources
            }
          />
          {props.viewportMode === "wide" || props.isImmersive ? (
            <>
              <LayoutPanel
                className="reader-immersive-panel reader-immersive-source-panel"
                data-keyboard-open={props.openImmersivePanel === "sources" ? "true" : undefined}
                data-hover-suppressed={props.suppressedImmersivePanel === "sources" ? "true" : undefined}
                padding={0}
                role="navigation"
                label={i18n._("reader.sources")}
                resizable={props.viewportMode === "wide" ? props.sourcesResizable : undefined}
                width={280}
                onFocusCapture={() => {
                  if (props.isImmersive) props.onOpenImmersivePanel("sources")
                }}
              >
                {props.sourceTree}
              </LayoutPanel>
              <ResizeHandle
                className="reader-workspace-resize"
                hasDivider
                label={i18n._("reader.resizeSources")}
                resizable={props.sourcesResizable}
              />
            </>
          ) : null}
          <ImmersiveEdge
            side="queue"
            label={i18n._("reader.revealQueue")}
            onOpen={() => props.onOpenImmersivePanel("queue")}
          />
          <LayoutPanel
            className="reader-immersive-panel reader-immersive-queue-panel"
            data-keyboard-open={props.openImmersivePanel === "queue" ? "true" : undefined}
            data-hover-suppressed={props.suppressedImmersivePanel === "queue" ? "true" : undefined}
            padding={0}
            role="region"
            label={i18n._("reader.queue")}
            aria-busy={props.queueStatus === "loading"}
            resizable={props.viewportMode === "wide" ? props.queueResizable : undefined}
            width={380}
            onFocusCapture={() => {
              if (props.isImmersive) props.onOpenImmersivePanel("queue")
            }}
          >
            {props.queuePane}
          </LayoutPanel>
          {props.viewportMode === "wide" ? (
            <ResizeHandle
              className="reader-workspace-resize"
              hasDivider
              label={i18n._("reader.resizeQueue")}
              resizable={props.queueResizable}
            />
          ) : null}
        </>
      )}
      content={(
        <LayoutContent
          ref={props.articlePanelRef}
          className="reader-workspace-article"
          padding={0}
          role="complementary"
          label={i18n._("reader.article")}
          aria-busy={props.detailStatus === "loading"}
          tabIndex={-1}
        >
          {props.articlePane}
        </LayoutContent>
      )}
    />
  )
}

interface ImmersiveEdgeProps {
  side: "sources" | "queue"
  label: string
  onOpen: () => void
}

function ImmersiveEdge(props: ImmersiveEdgeProps) {
  return (
    <div
      className={`reader-immersive-edge reader-immersive-${props.side}-edge`}
    >
      <button
        type="button"
        className="reader-immersive-edge-trigger"
        aria-label={props.label}
        onFocus={() => {
          props.onOpen()
        }}
        onClick={props.onOpen}
      >
        <span aria-hidden="true" />
      </button>
    </div>
  )
}
