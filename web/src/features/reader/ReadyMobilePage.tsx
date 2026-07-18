import type { ReaderController } from "./model/useReaderController"
import type { PreferencesController } from "../preferences/model/usePreferencesController"
import { ReaderRoutes } from "./routes/ReaderRoutes"

interface ReadyMobilePageProps {
  controller: ReaderController
  preferencesController: PreferencesController
  username: string
  sessionError: string | null
  onLogout: () => Promise<void>
}

export function ReadyMobilePage(props: ReadyMobilePageProps) {
  return <ReaderRoutes {...props} viewportMode="compact" />
}
