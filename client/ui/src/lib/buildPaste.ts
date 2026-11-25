// What a paste into a builds channel adds (#618, spec §4.3): an image (a screenshot), a link (one http(s) URL) or a
// title (any other text). An image wins over text pasted with it.

import type { StagedFile } from './tauri'

/** What quick add's "Add details" hands the full form. */
export type QuickPrefill = { title: string; link: string; files: StagedFile[] }

export type BuildPaste = { kind: 'image'; file: File } | { kind: 'link'; url: string } | { kind: 'text'; text: string }

/** A title's most characters (the server's limit). */
export const TITLE_MAX = 120

/** Typed (or pasted) text: a single http(s) URL is a link; other text a title, trimmed, its first 120 characters. */
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
  return { kind: 'text', text: [...t].slice(0, TITLE_MAX).join('') }
}

export function classifyPaste(data: DataTransfer): BuildPaste | null {
  const file = Array.from(data.files ?? []).find((f) => f.type.startsWith('image/'))
  if (file) return { kind: 'image', file }
  return classifyText(data.getData('text/plain') ?? '')
}
