import type { ReaderController } from "./model/useReaderController"
import { ReaderRoutes } from "./routes/ReaderRoutes"

interface ReadyMobilePageProps {
  controller: ReaderController
  username: string
  sessionError: string | null
  onLogout: () => Promise<void>
}

export function ReadyMobilePage(props: ReadyMobilePageProps) {
  return <ReaderRoutes {...props} viewportMode="compact" />
}
