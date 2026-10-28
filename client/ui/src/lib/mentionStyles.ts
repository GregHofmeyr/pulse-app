// Mention pills in rendered messages take the mentioned person's name style (plan reconciliation 10).
// Styles are set on DOM the sanitiser already produced; the sanitiser itself is unchanged.
import type { AppState } from './state'

const WRAP = 'mention-wrap'

let memo: { people: AppState['people']; members: AppState['members']; me: AppState['me']; look: (name: string) => string | null } | null = null

/** The user a pill's `@name` points at: matched case-insensitively against usernames, as pills are
 * made (people first, then members, then me). Built once per people/members/me: the same function
 * comes back until one of them changes, so it can feed a `$derived` without waking it. */
export function mentionLookup(s: AppState): (name: string) => string | null {
  if (memo && memo.people === s.people && memo.members === s.members && memo.me === s.me) return memo.look
  const ids = new Map<string, string>()
  const add = (username: string, id: string) => {
    const k = username.toLowerCase()
    if (!ids.has(k)) ids.set(k, id)
  }
  for (const p of Object.values(s.people)) add(p.user.username, p.user.id)
  for (const list of Object.values(s.members)) for (const m of list) add(m.user.username, m.user.id)
  if (s.me) add(s.me.username, s.me.id)
  const look = (name: string) => ids.get(name.toLowerCase()) ?? null
  memo = { people: s.people, members: s.members, me: s.me, look }
  return look
}

/** The name a `.mention` pill shows, without its `@`. */
export function mentionName(pill: Element): string {
  return (pill.textContent ?? '').replace(/^@/, '')
}

/**
 * Sets each known `.mention` pill's style to its person's name CSS (`css`); unknown names are left
 * alone. A gradient clips its background to the text, so the pill background moves to a wrapper.
 * Safe to run again on the same DOM (a profile changed): it restyles, wraps or unwraps as needed.
 */
export function styleMentions(root: HTMLElement, lookup: (name: string) => string | null, css: (userId: string) => string): void {
  for (const pill of root.querySelectorAll<HTMLElement>('.mention')) {
    const id = lookup(mentionName(pill))
    if (!id) continue
    const style = css(id)
    const gradient = style.includes('background-clip: text')
    const wrap = pill.parentElement?.classList.contains(WRAP) ? pill.parentElement : null
    pill.setAttribute('style', gradient ? `${style}; background-color: transparent; padding: 0` : style)
    if (gradient && !wrap) {
      const w = document.createElement('span')
      w.className = WRAP
      pill.replaceWith(w)
      w.append(pill)
    } else if (!gradient && wrap) {
      wrap.replaceWith(pill)
    }
  }
}
