// Mention pills in rendered messages take the mentioned person's name style (plan reconciliation 10).
// Styles are set on DOM the sanitiser already produced; the sanitiser itself is unchanged.
import { mentionSpans } from './composerDom'
import type { AppState } from './state'
import { setMentionLabel } from './mentionLabel'

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

/** The username a `.mention` pill was written with, without its `@` (kept aside once the pill shows
 *  a name instead). */
export function mentionName(pill: Element): string {
  return (pill as HTMLElement).dataset?.name ?? (pill.textContent ?? '').replace(/^@/, '')
}

/** `text` with each known `@username` (read as the server reads mentions) written as `@` + `label`
 *  of its person: for plain-text places such as notifications and previews (#47: the username
 *  never shows). */
export function showMentions(text: string, lookup: (name: string) => string | null, label: (userId: string) => string): string {
  let out = ''
  let at = 0
  for (const m of mentionSpans(text)) {
    const id = lookup(m.username)
    if (!id) continue
    out += `${text.slice(at, m.start)}@${label(id)}`
    at = m.end
  }
  return out + text.slice(at)
}

/**
 * Sets each known `.mention` pill's style to its person's name CSS (`css`) and makes it a focusable
 * button; unknown names are left alone (and not focusable). A gradient clips its background to the text, so the pill background moves to a wrapper.
 * Given `label`, a known pill shows `@` + the name that applies (#47) instead of the username it was
 * written with (kept in `data-name` for the lookup).
 * Safe to run again on the same DOM (a profile changed, a rename): it restyles, relabels, wraps or unwraps as needed.
 */
export function styleMentions(root: HTMLElement, lookup: (name: string) => string | null, css: (userId: string) => string, label?: (userId: string) => string): void {
  for (const pill of root.querySelectorAll<HTMLElement>('.mention')) {
    const name = mentionName(pill)
    const id = lookup(name)
    if (!id) {
      pill.removeAttribute('tabindex')
      pill.removeAttribute('role')
      continue
    }
    if (label) {
      pill.dataset.name = name
      const shown = label(id)
      // relabel only on a change: the pill's DOM is left alone otherwise
      if (pill.textContent !== `@${shown}` || !pill.querySelector('.at')) setMentionLabel(pill, shown)
    }
    // A known pill opens its person's card: reachable by keyboard too (Enter or Space, in MessageItem).
    pill.setAttribute('tabindex', '0')
    pill.setAttribute('role', 'button')
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
