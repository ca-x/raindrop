import { fireEvent, render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { expect, it, vi } from "vitest"

import { Providers } from "../../../app/Providers"
import { activateLocale } from "../../../shared/i18n/i18n"
import { initialReaderState } from "../model/reducer"
import { makeSubscription, subscriptionId } from "../model/testFixtures"
import { SourceTree } from "./SourceTree"

it("offers the primary subscription action from the empty state", async () => {
  activateLocale("en")
  const user = userEvent.setup()
  const onManage = vi.fn()
  render(
    <Providers>
      <SourceTree
        state={{
          ...initialReaderState,
          paneStatus: { ...initialReaderState.paneStatus, subscriptions: "ready" },
        }}
        onSelect={vi.fn()}
        onMoveSubscription={vi.fn(async () => true)}
        onManage={onManage}
        onPreferences={vi.fn()}
        onLogout={vi.fn()}
        density="balanced"
      />
    </Providers>,
  )

  await user.click(screen.getByRole("button", { name: "Add subscription" }))
  expect(onManage).toHaveBeenCalledOnce()
})

it("keeps selected-feed status in sources without subscription toolbar actions", () => {
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
        onMoveSubscription={vi.fn(async () => true)}
        onManage={vi.fn()}
        onPreferences={vi.fn()}
        onLogout={vi.fn()}
        density="balanced"
      />
    </Providers>,
  )

  expect(screen.queryByRole("button", { name: "Refresh Example Feed" })).not.toBeInTheDocument()
  expect(screen.queryByRole("button", { name: "Edit current subscription" })).not.toBeInTheDocument()
  expect(screen.getByText("Queued for refresh")).toBeVisible()
  fireEvent.click(screen.getByText("Queued for refresh"))
  expect(screen.getByText("Waiting for a refresh worker.")).toBeVisible()
})
