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
  updateSubscription,
} from "../api/subscriptions"
import {
  createCategory,
  deleteCategory,
  listCategories,
  updateCategory,
} from "../categories/api"

export interface ReaderApi {
  listSubscriptions: typeof listSubscriptions
  getSubscription: typeof getSubscription
  createSubscription: typeof createSubscription
  deleteSubscription: typeof deleteSubscription
  refreshSubscription: typeof refreshSubscription
  updateSubscription: typeof updateSubscription
  listCategories: typeof listCategories
  createCategory: typeof createCategory
  updateCategory: typeof updateCategory
  deleteCategory: typeof deleteCategory
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
  updateSubscription,
  listCategories,
  createCategory,
  updateCategory,
  deleteCategory,
  listEntries,
  getEntry,
  patchEntryState,
}
