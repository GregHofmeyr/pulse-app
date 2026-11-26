// What a paste into a builds channel adds (#618, spec §4.3): an image (a screenshot), a link (one http(s) URL) or a
// title (any other text). An image wins over text pasted with it.

import { BUILD_LIMITS } from './builds'
import type { StagedFile } from './tauri'

/** What quick add's "Add details" hands the full form. */
export type QuickPrefill = { title: string; link: string; files: StagedFile[] }

export type BuildPaste = { kind: 'image'; file: File } | { kind: 'link'; url: string } | { kind: 'text'; text: string }

/** Typed (or pasted) text: a single http(s) URL is a link; other text a title, trimmed, at most the server's title
 *  length (`BUILD_LIMITS.titleChars`). */
export function classifyText(raw: string): { kind: 'link'; url: string } | { kind: 'text'; text: string } | null {
  const t = raw.trim()
  if (!t) return null
  if (!/\s/.test(t) && /^https?:\/\//i.test(t)) {
    try {
      new URL(t)
      return { kind: 'link', url: t }
    } catch {
      /* not a URL after all: a title */
    }
  }
  return { kind: 'text', text: [...t].slice(0, BUILD_LIMITS.titleChars).join('') }
}

export function classifyPaste(data: DataTransfer): BuildPaste | null {
  const file = Array.from(data.files ?? []).find((f) => f.type.startsWith('image/'))
  if (file) return { kind: 'image', file }
  return classifyText(data.getData('text/plain') ?? '')
}
