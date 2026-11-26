// Custom typing text (#652): "<name> is <your text>" while one person types, the plain line otherwise. The line is
// worked out from who is typing now (state.typing), never from the order events came in, so starts, stops and
// timeouts in any order land on the same words.
import { charCount } from './profileDraft'

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

/** Zero-width spaces, joiners and marks, word joiners, the BOM and the Hangul fillers (names.rs `invisible`). */
const INVISIBLE = /[\u034f\u115f\u1160\u180e\u200b-\u200f\u2060-\u2064\u3164\ufeff\uffa0]/u
const drawn = (c: string | undefined) => !!c && !/\s/u.test(c) && !INVISIBLE.test(c)

/** Whether text reads as a link: a scheme, `www.`, or a domain (`pulse.app`). names.rs `has_link`. */
function hasLink(s: string): boolean {
  const t = s.toLowerCase()
  return t.includes('://') || t.includes('www.') || /[\p{L}\p{N}]\.[a-z]{2}/u.test(t)
}

/** Why typing text can't be saved, or null if it can (blank can: the plain "is typing"). The server's rules. */
export function typingTextProblem(text: string): string | null {
  const t = text.trim()
  if (!t) return null
  if (charCount(t) > TYPING_TEXT_MAX) return 'Typing text can be up to 32 characters.'
  if (/[\p{Cc}\u2028\u2029\u202a-\u202e\u2066-\u2069]/u.test(t)) return "Typing text can't contain line breaks or control characters."
  // no invisible characters, but a joiner between two drawn ones (inside an emoji) is part of the picture
  const cs = [...t]
  if (cs.some((c, i) => INVISIBLE.test(c) && !(c === '\u200d' && drawn(cs[i - 1]) && drawn(cs[i + 1])))) return "Typing text can't contain invisible characters."
  if (hasLink(t)) return "Typing text can't contain links."
  return null
}
