// `pulse://invite/<code>` links (#476): the invite page's "Open in Pulse". The core only passes on a plain code (the
// `invite-link` event, or `take_invite_link` for the link that started the app); the app shows that invite's card
// (signed in) or fills in the sign-up's invite code (signed out). Nothing joins on its own.
import { listen } from '@tauri-apps/api/event'
import { api } from './tauri'
import { wellFormedCode } from './serverInvites'

export const inviteLinks = $state({ code: null as string | null })

function opened(code: string | null | undefined) {
  if (code && wellFormedCode(code)) inviteLinks.code = code
}

/** Listen for invite links, then take the one that started the app, if any. */
export async function startInviteLinks(): Promise<() => void> {
  const off = await listen<string>('invite-link', (e) => opened(e.payload))
  opened(await api.takeInviteLink().catch(() => null))
  return off
}

/** A pasted invite link (`https://<server>/i/<code>`): the server's address and the code. Anything else: as typed. */
export function splitInviteLink(raw: string): { server: string | null; code: string } {
  const text = raw.trim()
  try {
    const url = new URL(text)
    const m = /^\/i\/([^/?#]+)\/?$/.exec(url.pathname)
    if ((url.protocol === 'https:' || url.protocol === 'http:') && m) return { server: url.origin, code: m[1] }
  } catch {
    // not a link
  }
  return { server: null, code: text }
}
