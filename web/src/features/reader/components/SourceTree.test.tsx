import { render, screen } from "@testing-library/react"
import { expect, it, vi } from "vitest"

import { Providers } from "../../../app/Providers"
import { activateLocale } from "../../../shared/i18n/i18n"
import { initialReaderState } from "../model/reducer"
import { makeSubscription, subscriptionId } from "../model/testFixtures"
import { SourceTree } from "./SourceTree"

it("disables redundant refresh while the selected feed is queued", () => {
  activateLocale("en")
  const subscription = makeSubscription({
    refresh: {
      operationId: "00000000-0000-4000-8000-000000000401",
      state: "PENDING",
      pendingState: "QUEUED",
      newCount: 0,
      updatedCount: 0,
      droppedCount: 0,
      entryIssues: [],
      generation: null,
      errorCode: null,
      retryAt: null,
      lastSuccessAt: null,
      queuedAt: "2026-07-18T02:00:00.000000Z",
      startedAt: null,
      completedAt: null,
    },
  })
  render(
    <Providers>
      <SourceTree
        state={{
          ...initialReaderState,
          selectedSource: { kind: "feed", feedId: subscription.feedId },
          subscriptionsById: { [subscriptionId]: subscription },
          subscriptionOrder: [subscriptionId],
        }}
        onSelect={vi.fn()}
        onManage={vi.fn()}
        onPreferences={vi.fn()}
        onRefresh={vi.fn()}
        onLogout={vi.fn()}
        density="balanced"
      />
    </Providers>,
  )

  expect(screen.getByRole("button", { name: "Refresh Example Feed" })).toHaveAttribute(
    "aria-disabled",
    "true",
  )
  expect(screen.getByText("Waiting for a refresh worker.")).toBeVisible()
})
