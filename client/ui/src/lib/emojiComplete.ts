// `:name` emoji autocomplete in the composer: what's being typed and how a pick rewrites the text.
// Kept apart from mentions.ts so @mentions stay untouched.

/** The partial shortcode after a `:` right before the caret (2+ word chars), or null. The `:` must
 * start the text or follow whitespace, so `http://xy` never opens the popup. */
export function emojiQuery(beforeCaret: string): string | null {
  const m = /(^|\s):([\w]{2,})$/.exec(beforeCaret)
  return m ? m[2] : null
}

/** Replace the `:partial` ending at `caret` with `insert` (Unicode or a custom token) plus a space. */
export function applyEmoji(text: string, caret: number, insert: string): { text: string; caret: number } {
  const q = emojiQuery(text.slice(0, caret)) ?? ''
  const start = caret - (q ? q.length + 1 : 0)
  return insertAt(text, start, caret, insert)
}

/** Put `insert` plus a space over the selection `start..end`; returns the new text and caret. */
export function insertAt(text: string, start: number, end: number, insert: string): { text: string; caret: number } {
  const s = insert + ' '
  return { text: text.slice(0, start) + s + text.slice(end), caret: start + s.length }
}
