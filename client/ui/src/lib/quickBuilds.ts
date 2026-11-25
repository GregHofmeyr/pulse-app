// Builds in the quick switcher (#619, spec §3.4): typing searches builds across your servers (a slow section, as
// messages are, lib/quickMessages.ts). `builds:` first limits the switcher to builds; the word(s) after it pick a game
// when they name exactly one by the start of its name or its initials (`builds: tfd` → The First Descendant).
import { nameKey } from './quotes'
import { isBuildsOnly, parseQuery } from './quickSwitch'
import { hasSearchableWords } from './searchQuery'
import type { Game } from './protocol/Game'
import type { Message } from './protocol/Message'

const BUILDS_PREFIX = 'builds:'

export type BuildsQuery = { only: boolean; gameKey: string | null; text: string }

/** A game name's initials ("The First Descendant" → "tfd"). */
const initials = (key: string): string => key.split(' ').map((w) => w[0] ?? '').join('')

/** The one game key `words` names, or null: none, or more than one (ambiguous). A full name wins outright (so
 *  "destiny" picks Destiny beside Destiny 2); else the start of a name, or (one word) a game's whole initials. */
function gameFor(words: string, keys: string[]): string | 'many' | null {
  if (keys.includes(words)) return words
  const hits = keys.filter((k) => k.startsWith(words) || (!words.includes(' ') && initials(k) === words))
  return hits.length === 0 ? null : hits.length === 1 ? hits[0] : 'many'
}

/** What `raw` searches builds for: null after `>` (actions). After `builds:` the longest run of first words that
 *  names exactly one game (`games`' names, as keys: the same game in two servers is one) filters by it and the rest is
 *  text; a run naming several is searched as text whole. A plain query is text only. */
export function parseBuildsQuery(raw: string, games: Game[]): BuildsQuery | null {
  if (!isBuildsOnly(raw)) {
    const q = parseQuery(raw)
    return q.actions ? null : { only: false, gameKey: null, text: q.text }
  }
  const rest = raw.trimStart().slice(BUILDS_PREFIX.length).trim()
  const words = rest.split(/\s+/).filter(Boolean)
  const keys = [...new Set(games.map((g) => nameKey(g.name)).filter(Boolean))]
  for (let n = words.length; n > 0; n--) {
    const found = gameFor(nameKey(words.slice(0, n).join(' ')), keys)
    if (found === 'many') break
    if (found) return { only: true, gameKey: found, text: words.slice(n).join(' ') }
  }
  return { only: true, gameKey: null, text: words.join(' ') }
}

/** What to ask the search route (`q`, `game` key), or null: a game alone is enough; text needs a letter or digit (the
 *  index holds words) and two characters after `builds:`, three in a plain query (builds share the server's search
 *  rate limit with message search, so a plain query asks only once it's more than a couple of letters). */
export function buildsQuery(p: BuildsQuery | null): { q: string; game: string | null } | null {
  if (!p) return null
  const textOk = p.text.length >= (p.only ? 2 : 3) && hasSearchableWords(p.text)
  if (p.gameKey) return { q: textOk ? p.text : '', game: p.gameKey }
  return textOk ? { q: p.text, game: null } : null
}

/** Current builds first, then archived ones (the server sends them so; kept here too), each group in its order. */
export function sortBuildHits(list: Message[]): Message[] {
  return [...list.filter((m) => !m.build?.archived_at), ...list.filter((m) => !!m.build?.archived_at)]
}

/** At most this many builds are listed. */
export const BUILDS_SHOWN = 8
