// Which entries the right-click menu for a person or stream shows (tested in participantMenu.test.ts).

export type MenuItemId =
  | 'volume' | 'local-mute' | 'sound-mute' | 'message' | 'stream-volume' | 'watch' | 'stop-watching' | 'full-view'
  | 'mute' | 'deafen' | 'go-live' | 'stop-streaming' | 'voice-settings'
  | 'server-mute' | 'server-deafen' | 'disconnect'

export type MenuCtx = {
  /** The menu is for you. */
  self: boolean
  /** The person is streaming in this channel. */
  live: boolean
  /** You're watching their stream. */
  watching: boolean
  /** ...and it's in full view. */
  full: boolean
  /** You're connected to this channel's voice. */
  inCall: boolean
  canGoLive: boolean
  streamingSelf: boolean
  /** The call is a server voice channel's (#140): anyone there may server mute, deafen or disconnect anyone else. */
  serverCall?: boolean
}

export function menuItems(c: MenuCtx): MenuItemId[] {
  if (c.self) {
    const out: MenuItemId[] = ['mute', 'deafen']
    if (c.streamingSelf) out.push('stop-streaming')
    else if (c.canGoLive) out.push('go-live')
    out.push('voice-settings')
    return out
  }
  const out: MenuItemId[] = ['volume', 'local-mute', 'sound-mute', 'message']
  if (c.live) {
    if (c.watching) out.push('stream-volume')
    out.push(c.watching ? 'stop-watching' : 'watch')
    if (c.watching && !c.full) out.push('full-view')
  }
  if (c.serverCall) out.push('server-mute', 'server-deafen', 'disconnect')
  return out
}

/** The server voice items in a person's menu away from the call (member list, message author, profile card; #144).
 *  Undoing works wherever they are, so a held mute or deafen offers its undo even once they've left voice; muting needs
 *  them in this server's voice, so the other way round shows only then. Disconnect stays in the call's own menu. Never
 *  for yourself (someone else has to undo it) or outside a server. */
export function memberVoiceItems(c: {
  self: boolean
  server: boolean
  /** They're in one of this server's voice channels. */
  inVoice: boolean
  muted: boolean
  deafened: boolean
}): ('server-mute' | 'server-deafen')[] {
  if (c.self || !c.server) return []
  const out: ('server-mute' | 'server-deafen')[] = []
  if (c.inVoice || c.muted) out.push('server-mute')
  if (c.inVoice || c.deafened) out.push('server-deafen')
  return out
}

/** Where a menu opens: at the pointer, or (from the keyboard) under the element that asked. */
export function menuPoint(e: MouseEvent | KeyboardEvent, anchor: HTMLElement): { x: number; y: number } {
  // a click with no detail is a key press on a button: it has no pointer to open at
  if (e instanceof MouseEvent && !(e.type === 'click' && e.detail === 0)) return { x: e.clientX, y: e.clientY }
  const r = anchor.getBoundingClientRect()
  return { x: r.left, y: r.bottom }
}
