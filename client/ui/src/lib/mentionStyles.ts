// Mention pills in rendered messages take the mentioned person's name style (plan reconciliation 10).
// Styles are set on DOM the sanitiser already produced; the sanitiser itself is unchanged.
import type { AppState } from './state'

const WRAP = 'mention-wrap'

/** The user a pill's `@name` points at: matched case-insensitively against usernames, as pills are made. */
export function mentionLookup(s: AppState): (name: string) => string | null {
  return (name) => {
    const n = name.toLowerCase()
    for (const p of Object.values(s.people)) if (p.user.username.toLowerCase() === n) return p.user.id
    for (const list of Object.values(s.members)) {
      const m = list.find((x) => x.user.username.toLowerCase() === n)
      if (m) return m.user.id
    }
    return s.me?.username.toLowerCase() === n ? s.me.id : null
  }
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
