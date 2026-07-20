import type { ReaderController } from "./model/useReaderController"
import type { AiSettingsController } from "../ai/model/useAiSettingsController"
import type { PreferencesController } from "../preferences/model/usePreferencesController"
import { ReaderRoutes } from "./routes/ReaderRoutes"

interface ReadyMobilePageProps {
  controller: ReaderController
  preferencesController: PreferencesController
  aiSettingsController?: AiSettingsController
  username: string
  sessionError: string | null
  onLogout: () => Promise<void>
  onUnauthenticated?: () => void
}

export function ReadyMobilePage(props: ReadyMobilePageProps) {
  return <ReaderRoutes {...props} viewportMode="compact" />
}
