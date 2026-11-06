import { apiRequest, invalidResponseError } from "../../../shared/api/client"
import { isArticleQuestionResponse, type ArticleQuestionRequest, type ArticleQuestionResponse } from "./questions.generated"

export async function askArticleQuestion(entryId: string, csrfToken: string, request: ArticleQuestionRequest, signal?: AbortSignal): Promise<ArticleQuestionResponse> {
  const result = await apiRequest(`/api/v1/entries/${encodeURIComponent(entryId)}/ai/questions`, {
    method: "POST",
    headers: { "x-csrf-token": csrfToken },
    body: JSON.stringify(request),
    signal,
  })
  if (!isArticleQuestionResponse(result)) throw invalidResponseError()
  return result
}
