// Invite cards' previews (#80), one per code for the whole session, and the signed-in server's address that invite
// links are made from and recognised by. The server rate-limits code lookups (and an unknown code counts towards a
// block), so a code is asked for once: a card mounting again, scrolling back or re-rendering reads the cache, and only
// a failed lookup (rate limited, offline) may be asked again, and not before a few minutes have passed.
import type { InvitePreview } from './protocol/InvitePreview'
import type { CardState } from './serverInvites'
import { api, errorText } from './tauri'

export type PreviewEntry =
  | { status: 'loading' }
  | { status: 'ok'; preview: InvitePreview }
  /** An unknown or revoked code (404): it stays that way. */
  | { status: 'gone' }
  /** Couldn't ask just now: asked again by a card mounting after `RETRY_MS`. */
  | { status: 'error'; text: string; at: number }

/** How long a failed lookup waits before a card may ask again. */
export const RETRY_MS = 5 * 60_000

export const invitePreviews = $state<{ byCode: Record<string, PreviewEntry> }>({ byCode: {} })

/** Ask for `code`'s preview unless it's known, on its way, or failed less than [`RETRY_MS`] ago. */
export function loadInvitePreview(code: string, now = Date.now()): void {
  const had = invitePreviews.byCode[code]
  if (had && !(had.status === 'error' && now - had.at >= RETRY_MS)) return
  invitePreviews.byCode[code] = { status: 'loading' }
  api.invitePreview(code).then(
    (preview) => {
      invitePreviews.byCode[code] = { status: 'ok', preview }
    },
    (e) => {
      const text = errorText(e)
      invitePreviews.byCode[code] = text === 'not found' ? { status: 'gone' } : { status: 'error', text, at: Date.now() }
    },
  )
}

/** What an accept (or a failed one) found out about `code`: the card shows it without asking again. */
export function noteInviteState(code: string, state: CardState): void {
  const had = invitePreviews.byCode[code]
  if (state === 'not_found' || state === 'revoked') invitePreviews.byCode[code] = { status: 'gone' }
  else if (had?.status === 'ok' && state !== 'unavailable') invitePreviews.byCode[code] = { status: 'ok', preview: { ...had.preview, state } }
}

/** The signed-in server's address, once asked (null until then, or signed out). */
export const serverOrigin = $state({ origin: null as string | null })
let asking: Promise<void> | null = null

/** Asks the core once per sign-in; a failure is asked again by the next caller. */
export function loadServerOrigin(): void {
  if (asking || serverOrigin.origin) return
  const p: Promise<void> = api.serverOrigin().then(
    (o) => {
      if (asking === p) serverOrigin.origin = o ?? null
    },
    () => {
      if (asking === p) asking = null
    },
  )
  asking = p
}

/** Sign-out or an account switch: the next account sees its own cards, maybe on another server. */
export function forgetInvitePreviews(): void {
  invitePreviews.byCode = {}
  serverOrigin.origin = null
  asking = null
}
