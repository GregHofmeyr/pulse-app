// Custom typing text (#652): "<name> is <your text>" while one person types, the plain line otherwise. The line is
// worked out from who is typing now (state.typing), never from the order events came in, so starts, stops and
// timeouts in any order land on the same words.
import { charCount } from './profileDraft'

/** How long after someone's message a Typing from them is taken as stale: sent just before it, delivered just after
 *  (state.ts `typingStops`). Short, as a real new start lands in it too: the sender's next Typing after a send waits
 *  until just past it (markdown.ts `typingSentAfterSend`), so it isn't dropped. */
export const TYPING_STOP_GUARD_MS = 400

/** Most characters in typing text (the server's limit, server/src/profiles/names.rs). */
export const TYPING_TEXT_MAX = 32

/** Tap-to-use typing text, under the field in Profile settings. */
export const TYPING_SUGGESTIONS = ['cooking', 'yapping', 'plotting', 'barking', 'typing a novel', 'scheming'] as const

/** Someone typing, as the line names them: their name here and their typing text (null: none). */
export type Typer = { id: string; name: string; custom: string | null }

/** The words on the typing line: the names (bold) and the rest. `key` changes whenever the words do. */
export type TypingLine = { key: string; names: string; rest: string }

export function typingLine(typers: readonly Typer[]): TypingLine | null {
  if (typers.length === 0) return null
  const names = typers.map((t) => t.name).join(', ')
  // one person: their own words; two or more: the plain line, whatever anyone's typing text
  const rest = typers.length === 1 ? `is ${typers[0].custom ?? 'typing…'}` : 'are typing…'
  return { key: `${names}\u0000${rest}`, names, rest }
}

/** A `typingLine` key tracker for a `$derived` (like `keepStable`): whether a new line replaces one already on screen
 *  (true: the words crossfade) rather than appearing from nothing (false: it rises in). */
export function swapTracker(): (key: string | null) => boolean {
  let last: string | null = null
  return (key) => {
    const swap = last !== null && key !== null && key !== last
    last = key
    return swap
  }
}

/** A format character (Cf) or one that draws nothing: zero-width spaces, joiners and marks, word joiners, the BOM, the
 *  Hangul fillers, the soft hyphen, the braille blank, the Mongolian variation selectors (names.rs `hides`). */
const HIDES = /[\p{Cf}\u00ad\u034f\u115f\u1160\u180b-\u180f\u200b-\u200f\u2060-\u2064\u2800\u3164\ufeff\uffa0]/u
const SPACE = /\p{White_Space}/u
const drawn = (c: string | undefined) => !!c && !SPACE.test(c) && !HIDES.test(c)
const TAG = /[\u{e0020}-\u{e007f}]/u

/** Whether text holds a character that hides: any HIDES one, but a joiner between two drawn characters (an emoji's
 *  parts) and a run of tag characters straight after 🏴 (a subdivision flag) are allowed. names.rs `has_hidden`. */
function hasHidden(s: string): boolean {
  const cs = [...s]
  let flag = false
  return cs.some((c, i) => {
    flag = TAG.test(c) && (flag || cs[i - 1] === '\u{1f3f4}')
    return HIDES.test(c) && !flag && !(c === '\u200d' && drawn(cs[i - 1]) && drawn(cs[i + 1]))
  })
}

/** Whether text reads as a link: a scheme, `www.`, or a domain: a letter or digit, a dot or a lookalike, then two
 *  letters of any script (`pulse.app`, `пример.рф`; `brain.exe` too, which is fine). names.rs `has_link`, class for
 *  class (Unicode L and N; ASCII-only lowercasing). */
function hasLink(s: string): boolean {
  const t = s.replace(/[\u3002\uff0e\uff61\u2024\u2027\ufe52]/gu, '.').replace(/[A-Z]/g, (c) => c.toLowerCase())
  return t.includes('://') || t.includes('www.') || /[\p{L}\p{N}]\.\p{L}\p{L}/u.test(t)
}

/** Why typing text can't be saved, or null if it can (blank can: the plain "is typing"). The server's rules. */
export function typingTextProblem(text: string): string | null {
  const t = text.trim()
  if (!t) return null
  if (charCount(t) > TYPING_TEXT_MAX) return 'Typing text can be up to 32 characters.'
  if (/[\p{Cc}\u2028\u2029\u202a-\u202e\u2066-\u2069]/u.test(t)) return "Typing text can't contain line breaks or control characters."
  if (hasHidden(t)) return "Typing text can't contain invisible characters."
  if (hasLink(t)) return "Typing text can't contain links."
  return null
}
