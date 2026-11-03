// @-mention autocomplete: what's being typed, who matches, and how inserting a name rewrites the text.

/** Someone who can be @mentioned: `name` is the name shown here, `username` what a mention is written
 *  as (the composer shows it as a chip with `name`), `alts` every other name they can be found by. */
export type Mentionable = { id: string; name: string; username: string; alts: string[] }

/** The partial name after an `@` right before the caret (`''` for a bare `@`), or null. */
export function mentionQuery(beforeCaret: string): string | null {
  const m = /(^|\s)@([\w.-]*)$/.exec(beforeCaret)
  return m ? m[2] : null
}

/** Whether `name`, or one of its words, starts with `lq` (lowercase). */
const startsWith = (name: string, lq: string) => {
  const n = name.toLowerCase()
  return n.startsWith(lq) || n.split(/\s+/).some((w) => w.startsWith(lq))
}

/** People one of whose names (shown, username, the others) starts with `q` (case-insensitive), at
 *  most 6. */
export function suggest<T extends { name: string; username?: string; alts?: string[] }>(people: T[], q: string): T[] {
  const lq = q.toLowerCase()
  return people.filter((p) => [p.name, p.username ?? '', ...(p.alts ?? [])].some((n) => n && startsWith(n, lq))).slice(0, 6)
}

/** Who matches `q` anywhere in their name or other names (`alt`, `alts`: a username or nicknames
 *  behind the name shown), case-insensitive: names that start with it first, then by how early it
 *  appears, then A to Z. At most `max`. For the search box, where you may only remember part of a name. */
export function rankNames<T extends { name: string; alt?: string; alts?: string[] }>(items: T[], q: string, max = 8): T[] {
  const lq = q.toLowerCase()
  const at = (x: T) => {
    const found = [x.name, x.alt ?? '', ...(x.alts ?? [])].filter((n) => n).map((n) => n.toLowerCase().indexOf(lq)).filter((p) => p >= 0)
    return found.length ? Math.min(...found) : -1
  }
  return items
    .map((x) => ({ x, p: at(x) }))
    .filter((e) => e.p >= 0)
    .sort((a, b) => a.p - b.p || a.x.name.toLowerCase().localeCompare(b.x.name.toLowerCase()))
    .slice(0, max)
    .map((e) => e.x)
}

/** Replace the `@partial` ending at `caret` with `@name `; returns the new text and caret. */
export function applyMention(text: string, caret: number, name: string): { text: string; caret: number } {
  const before = text.slice(0, caret)
  const q = mentionQuery(before) ?? ''
  const start = caret - q.length - 1
  const inserted = `@${name} `
  return { text: text.slice(0, start) + inserted + text.slice(caret), caret: start + inserted.length }
}
