export const MAX_LOOKUP_SELECTION_CHARACTERS = 200
export const MAX_TRANSLATION_SELECTION_CHARACTERS = 8_000

export function readSelectedArticleText(
  container: HTMLElement | null,
  selection: Selection | null = document.getSelection(),
): string {
  if (
    !container ||
    !selection ||
    selection.isCollapsed ||
    selection.rangeCount === 0
  ) {
    return ""
  }
  for (let index = 0; index < selection.rangeCount; index += 1) {
    const range = selection.getRangeAt(index)
    if (
      !container.contains(range.startContainer) ||
      !container.contains(range.endContainer)
    ) {
      return ""
    }
  }
  const text = unicodePrefix(
    selection.toString().trim(),
    MAX_TRANSLATION_SELECTION_CHARACTERS + 1,
  )
  return Array.from(text).some(
    (character) =>
      isControlCharacter(character) &&
      character !== "\n" &&
      character !== "\r" &&
      character !== "\t",
  )
    ? ""
    : text
}

export function unicodeLength(value: string): number {
  return Array.from(value).length
}

export function selectionExcerpt(value: string): string {
  const compact = value.replace(/\s+/gu, " ").trim()
  return unicodeLength(compact) > 180
    ? `${unicodePrefix(compact, 179)}…`
    : compact
}

function unicodePrefix(value: string, maximum: number): string {
  return Array.from(value).slice(0, maximum).join("")
}

function isControlCharacter(character: string): boolean {
  return /\p{Cc}/u.test(character)
}
