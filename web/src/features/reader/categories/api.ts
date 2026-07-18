import { apiRequest, invalidResponseError } from "../../../shared/api/client"
import {
  isCategoryListResponse,
  isCategoryResponse,
  type CategoryListResponse,
  type CategoryResponse,
  type CreateCategoryRequest,
  type UpdateCategoryRequest,
} from "../api/organization.generated"

export async function listCategories(signal?: AbortSignal): Promise<CategoryListResponse> {
  const response = await apiRequest("/api/v1/categories", { signal })
  if (!isCategoryListResponse(response)) throw invalidResponseError()
  return response
}

export async function createCategory(
  request: CreateCategoryRequest,
  csrfToken: string,
  signal?: AbortSignal,
): Promise<CategoryResponse> {
  const response = await apiRequest("/api/v1/categories", {
    method: "POST",
    headers: csrfHeaders(csrfToken),
    body: JSON.stringify(request),
    signal,
  })
  if (!isCategoryResponse(response)) throw invalidResponseError()
  return response
}

export async function updateCategory(
  categoryId: string,
  request: UpdateCategoryRequest,
  csrfToken: string,
  signal?: AbortSignal,
): Promise<CategoryResponse> {
  const response = await apiRequest(categoryPath(categoryId), {
    method: "PATCH",
    headers: csrfHeaders(csrfToken),
    body: JSON.stringify(request),
    signal,
  })
  if (!isCategoryResponse(response)) throw invalidResponseError()
  return response
}

export async function deleteCategory(
  categoryId: string,
  csrfToken: string,
  signal?: AbortSignal,
): Promise<void> {
  const response = await apiRequest(categoryPath(categoryId), {
    method: "DELETE",
    headers: csrfHeaders(csrfToken),
    signal,
  })
  if (response !== undefined) throw invalidResponseError()
}

function categoryPath(categoryId: string): string {
  return `/api/v1/categories/${encodeURIComponent(categoryId)}`
}

function csrfHeaders(csrfToken: string): HeadersInit {
  return { "x-csrf-token": csrfToken }
}
