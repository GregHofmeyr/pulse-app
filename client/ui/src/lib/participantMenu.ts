// Which entries the right-click menu for a person or stream shows (tested in participantMenu.test.ts).

export type MenuItemId =
  | 'volume' | 'local-mute' | 'message' | 'stream-volume' | 'watch' | 'stop-watching' | 'full-view'
  | 'mute' | 'deafen' | 'go-live' | 'stop-streaming' | 'voice-settings'

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
}

export function menuItems(c: MenuCtx): MenuItemId[] {
  if (c.self) {
    const out: MenuItemId[] = ['mute', 'deafen']
    if (c.streamingSelf) out.push('stop-streaming')
    else if (c.canGoLive) out.push('go-live')
    out.push('voice-settings')
    return out
  }
  const out: MenuItemId[] = ['volume', 'local-mute', 'message']
  if (c.live) {
    if (c.watching) out.push('stream-volume')
    out.push(c.watching ? 'stop-watching' : 'watch')
    if (c.watching && !c.full) out.push('full-view')
  }
  return out
}

/** Where a menu opens: at the pointer, or (from the keyboard) under the element that asked. */
export function menuPoint(e: MouseEvent | KeyboardEvent, anchor: HTMLElement): { x: number; y: number } {
  // a click with no detail is a key press on a button: it has no pointer to open at
  if (e instanceof MouseEvent && !(e.type === 'click' && e.detail === 0)) return { x: e.clientX, y: e.clientY }
  const r = anchor.getBoundingClientRect()
  return { x: r.left, y: r.bottom }
}
