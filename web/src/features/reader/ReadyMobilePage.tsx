import type { ReaderController } from "./model/useReaderController"
import type { AiSettingsController } from "../ai/model/useAiSettingsController"
import type { PreferencesController } from "../preferences/model/usePreferencesController"
import type { ProfileController } from "../profile/model/useProfileController"
import type { TranslationSettingsController } from "../translation/model/useTranslationSettingsController"
import type { BackupController } from "../backups/model/useBackupController"
import { ReaderRoutes } from "./routes/ReaderRoutes"

interface ReadyMobilePageProps {
  controller: ReaderController
  preferencesController: PreferencesController
  profileController?: ProfileController
  aiSettingsController?: AiSettingsController
  translationController?: TranslationSettingsController
  backupController?: BackupController
  username: string
  sessionError: string | null
  onLogout: () => Promise<void>
  onUnauthenticated?: () => void
}

export function ReadyMobilePage(props: ReadyMobilePageProps) {
  return <ReaderRoutes {...props} viewportMode="compact" />
}
