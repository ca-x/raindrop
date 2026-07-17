import {
  getEntry,
  listEntries,
  patchEntryState,
} from "../api/entries"
import {
  createSubscription,
  deleteSubscription,
  getSubscription,
  listSubscriptions,
  refreshSubscription,
} from "../api/subscriptions"

export interface ReaderApi {
  listSubscriptions: typeof listSubscriptions
  getSubscription: typeof getSubscription
  createSubscription: typeof createSubscription
  deleteSubscription: typeof deleteSubscription
  refreshSubscription: typeof refreshSubscription
  listEntries: typeof listEntries
  getEntry: typeof getEntry
  patchEntryState: typeof patchEntryState
}

export const defaultReaderApi: ReaderApi = {
  listSubscriptions,
  getSubscription,
  createSubscription,
  deleteSubscription,
  refreshSubscription,
  listEntries,
  getEntry,
  patchEntryState,
}
