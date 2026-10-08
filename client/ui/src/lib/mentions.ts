// @-mention autocomplete: what's being typed, who matches, and how inserting a name rewrites the text.

export type Mentionable = { id: string; name: string }

/** The partial name after an `@` right before the caret (`''` for a bare `@`), or null. */
export function mentionQuery(beforeCaret: string): string | null {
  const m = /(^|\s)@([\w.-]*)$/.exec(beforeCaret)
  return m ? m[2] : null
}

/** People whose name starts with `q` (case-insensitive), at most 6. */
export function suggest(people: Mentionable[], q: string): Mentionable[] {
  const lq = q.toLowerCase()
  return people.filter((p) => p.name.toLowerCase().startsWith(lq)).slice(0, 6)
}

/** Replace the `@partial` ending at `caret` with `@name `; returns the new text and caret. */
export function applyMention(text: string, caret: number, name: string): { text: string; caret: number } {
  const before = text.slice(0, caret)
  const q = mentionQuery(before) ?? ''
  const start = caret - q.length - 1
  const inserted = `@${name} `
  return { text: text.slice(0, start) + inserted + text.slice(caret), caret: start + inserted.length }
}
