// Server invites (#80), the pure side: invite links (making one from the signed-in server's address, and finding them
// in a message), the Invite dialog's choices, and what an invite card says in each state.
import type { InvitePreview } from './protocol/InvitePreview'
import type { InviteState } from './protocol/InviteState'

/** A code as the server makes them: 10 characters, no 0/O, 1/l/I (server/src/servers/invites.rs `ALPHABET`). */
const CODE = '[a-km-zA-HJ-NP-Z2-9]{10}'
const CODE_RE = new RegExp(`^${CODE}$`)

/** Whether `code` could be one of the server's codes at all: anything else is never looked up (an unknown code counts
 *  towards a lookup block). */
export const wellFormedCode = (code: string): boolean => CODE_RE.test(code)

/** The link for invite `code` on the server at `origin` (the address the app is signed in to). */
export const inviteLink = (origin: string, code: string): string => `${origin.replace(/\/+$/, '')}/i/${code}`

/** The most invite cards one message shows. */
export const MAX_CARDS = 3

const escapeRe = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')

/** The invite codes linked in `text` (`https://<the server's host>/i/<code>`), first seen first, each once, at most
 *  [`MAX_CARDS`]. Links to another host, or with something that isn't a code after `/i/`, are left alone. */
export function inviteCodesIn(text: string, origin: string | null): string[] {
  if (!origin || !text.includes('/i/')) return []
  let host: string
  try {
    host = new URL(origin).host
  } catch {
    return []
  }
  if (!host) return []
  const re = new RegExp(`https?://(?:www\\.)?${escapeRe(host)}/i/(${CODE})(?![A-Za-z0-9])`, 'gi')
  const out: string[] = []
  for (const m of text.matchAll(re)) {
    // the host matched case-insensitively, the code must not be
    const code = m[0].slice(m[0].length - 10)
    if (wellFormedCode(code) && !out.includes(code)) out.push(code)
    if (out.length >= MAX_CARDS) break
  }
  return out
}

export type Choice = { value: number; label: string }

/** Expiry choices in seconds (`0` = never). Members are capped at 7 days by the server: "Never" is the owner's. */
export function expiryChoices(owner: boolean): Choice[] {
  return [
    { value: 3600, label: '1 hour' },
    { value: 86400, label: '1 day' },
    { value: 7 * 86400, label: '7 days' },
    ...(owner ? [{ value: 0, label: 'Never' }] : []),
  ]
}

/** Use limits (`0` = no limit). Members are capped at 25 by the server: "No limit" is the owner's. */
export function usesChoices(owner: boolean): Choice[] {
  return [
    { value: 1, label: '1' },
    { value: 5, label: '5' },
    { value: 25, label: '25' },
    ...(owner ? [{ value: 0, label: 'No limit' }] : []),
  ]
}

/** What a new link starts with: a day, one use (the server's own defaults). */
export const DEFAULT_EXPIRY = 86400
export const DEFAULT_USES = 1

/** The longest custom join button text (protocol `INVITE_JOIN_TEXT_CHARS`). */
export const JOIN_TEXT_MAX = 32

/** The join button's words: the invite's own, else "Join <server>". */
export const joinLabel = (name: string, joinText?: string | null): string => joinText?.trim() || `Join ${name}`

/** What a card can be in: the server's states, an invite it doesn't know (unknown or revoked: 404), or one it couldn't
 *  look up just now (rate limited, offline). */
export type CardState = InviteState | 'not_found' | 'unavailable'

export type CardView = {
  /** The line under the server's name when it no longer lets you in; null while it does. */
  note: string | null
  /** The button: Join, Open (you're in) or a disabled one; null for none. */
  button: { label: string; action: 'join' | 'open' | null } | null
  /** Greyed out: it doesn't let you in. */
  dead: boolean
}

/** How a card reads in `state`. `name`/`joinText`/`forName` come from the preview when there is one. */
export function cardView(state: CardState, p?: Pick<InvitePreview, 'name' | 'join_text' | 'for_user_name'>): CardView {
  const name = p?.name ?? 'this server'
  switch (state) {
    case 'valid':
      return { note: null, button: { label: joinLabel(name, p?.join_text), action: 'join' }, dead: false }
    case 'already_member':
      return { note: "You're in", button: { label: 'Open', action: 'open' }, dead: false }
    case 'expired':
      return { note: 'This invite has expired', button: null, dead: true }
    case 'used_up':
      return { note: 'This invite has been used', button: null, dead: true }
    case 'for_someone_else':
      return { note: `For ${p?.for_user_name ?? 'someone else'} only`, button: { label: joinLabel(name, p?.join_text), action: null }, dead: true }
    case 'banned':
      return { note: "You can't join this server", button: null, dead: true }
    case 'revoked':
    case 'not_found':
      return { note: 'Invite not found', button: null, dead: true }
    case 'unavailable':
      return { note: "Couldn't load this invite right now", button: null, dead: true }
  }
}

/** The state a failed accept says the invite is in, from the server's words; null when they say something else. */
export function stateFromError(text: string): CardState | null {
  const t = text.toLowerCase()
  if (t === 'not found') return 'not_found'
  if (t.includes('expired')) return 'expired'
  if (t.includes('used up')) return 'used_up'
  if (t.includes('for someone else')) return 'for_someone_else'
  if (t.includes("already in")) return 'already_member'
  if (t.includes("can't join")) return 'banned'
  return null
}

/** "N online · M members". */
export function countsLine(online: number, members: number): string {
  return `${online} online · ${members} ${members === 1 ? 'member' : 'members'}`
}

/** An invite's expiry for the Invites list: "Never", "Expired", or how long is left ("in 3 hours", "in 2 days"). */
export function expiresText(expiresAt: string | undefined, now: number): string {
  if (!expiresAt) return 'Never'
  const left = Date.parse(expiresAt) - now
  if (!(left > 0)) return 'Expired'
  const mins = Math.ceil(left / 60_000)
  if (mins < 60) return `in ${mins} ${mins === 1 ? 'minute' : 'minutes'}`
  const hours = Math.round(mins / 60)
  if (hours < 48) return `in ${hours} ${hours === 1 ? 'hour' : 'hours'}`
  const days = Math.round(hours / 24)
  return `in ${days} days`
}

/** An invite's uses for the Invites list: "2 / 5", or "2 / ∞" with no limit. */
export const usesText = (uses: number, max?: number): string => `${uses} / ${max ?? '∞'}`
