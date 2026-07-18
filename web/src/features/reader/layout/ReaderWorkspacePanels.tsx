import { Layout, LayoutContent, LayoutPanel } from "@astryxdesign/core/Layout"
import { ResizeHandle, type ResizableProps } from "@astryxdesign/core/Resizable"
import { useLingui } from "@lingui/react"
import type { ReactNode } from "react"

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
      height="fill"
      padding={0}
      start={(
        <>
          {props.viewportMode === "wide" ? (
            <>
              <LayoutPanel
                padding={0}
                role="navigation"
                label={i18n._("reader.sources")}
                resizable={props.sourcesResizable}
              >
                {props.sourceTree}
              </LayoutPanel>
              <ResizeHandle
                hasDivider
                label={i18n._("reader.resizeSources")}
                resizable={props.sourcesResizable}
              />
            </>
          ) : null}
          <LayoutPanel
            padding={0}
            role="region"
            label={i18n._("reader.queue")}
            aria-busy={props.queueStatus === "loading"}
            resizable={props.viewportMode === "wide" ? props.queueResizable : undefined}
            width={380}
          >
            {props.queuePane}
          </LayoutPanel>
          {props.viewportMode === "wide" ? (
            <ResizeHandle
              hasDivider
              label={i18n._("reader.resizeQueue")}
              resizable={props.queueResizable}
            />
          ) : null}
        </>
      )}
      content={(
        <LayoutContent
          padding={0}
          role="complementary"
          label={i18n._("reader.article")}
          aria-busy={props.detailStatus === "loading"}
        >
          {props.articlePane}
        </LayoutContent>
      )}
    />
  )
}
