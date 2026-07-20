import { vi } from "vitest"

import type { AiSettingsController } from "./useAiSettingsController"

export function fakeAiSettingsController(
  overrides: Partial<AiSettingsController> = {},
): AiSettingsController {
  return {
    csrfToken: "csrf-memory",
    providers: [
      {
        providerId: "00000000-0000-4000-8000-000000000101",
        scope: "USER",
        canEdit: true,
        displayName: "Primary model",
        kind: "OPENAI_RESPONSES",
        endpoint: "https://api.openai.com/",
        model: "gpt-5-mini",
        capabilities: { supportsUsage: true, supportsIdempotency: true },
        policy: {
          maxConcurrency: 2,
          requestsPerMinute: 60,
          maxInputTokensPerRequest: 32_768,
          maxOutputTokensPerRequest: 4096,
          inputCostMicrosPerMillionTokens: null,
          outputCostMicrosPerMillionTokens: null,
          maxCostMicrosPerRequest: 250_000,
        },
        isEnabled: true,
        revision: 0,
        createdAt: "2026-07-20T10:00:00Z",
        updatedAt: "2026-07-20T10:00:00Z",
      },
    ],
    keyringStatus: "AVAILABLE",
    configEnvelope: {
      pluginState: "READY",
      mcpState: "CONTRACT_READY_TRANSPORT_UNAVAILABLE",
      config: null,
    },
    loadStatus: "ready",
    error: null,
    isSavingProvider: false,
    isSavingConfig: false,
    load: vi.fn().mockResolvedValue(undefined),
    saveProvider: vi.fn().mockResolvedValue(true),
    saveConfig: vi.fn().mockResolvedValue(true),
    cancel: vi.fn(),
    clearError: vi.fn(),
    ...overrides,
  }
}
