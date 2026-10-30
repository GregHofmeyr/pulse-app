// Desktop notifications for messages: whether one goes out (the message sound's rule, while the
// window isn't focused), what it says, and how often. The Rust core only shows what this decides.
import type { AppState } from './state'
import type { Channel } from './protocol/Channel'
import type { Message } from './protocol/Message'
import { conversationName, shouldAlert, type AlertInput } from './conversations'
import { displayName } from './selectors'
import { TOKEN_RE } from './emoji'

/** What the core shows; a click comes back with `channelId` and `threadId`. */
export type Notice = { title: string; body: string; channelId: string; threadId?: string }
export type NotifyPrefs = { enabled: boolean; showText: boolean }

export const BODY_MAX = 140

/** The notification for a new message, or null when none should go out. `more`: how many earlier
 *  messages this one stands in for ("sam and 3 more"). */
export function notifyMessage(s: AppState, v: AlertInput, prefs: NotifyPrefs, more = 0): Notice | null {
  if (!prefs.enabled || v.focused || !shouldAlert(v)) return null
  const m = v.message
  const n: Notice = { title: notificationTitle(s, m, v.channel, more), body: notificationBody(m, v.me, prefs.showText), channelId: m.channel_id }
  if (m.thread_root_id) n.threadId = m.thread_root_id
  return n
}

/** "sam", "sam in Friends", "sam in #general (Main)", with " and N more" after the name. */
export function notificationTitle(s: AppState, m: Message, channel: Channel, more: number): string {
  const who = m.author_id ? displayName(s, channel.server_id, m.author_id) : 'Pulse'
  const many = more > 0 ? ` and ${more} more` : ''
  let where = ''
  if (channel.server_id) {
    const server = s.servers.find((x) => x.id === channel.server_id)
    where = ` in #${channel.name ?? ''}${server ? ` (${server.name})` : ''}`
  } else if (channel.kind === 'group') {
    where = ` in ${conversationName(s, channel.id)}`
  }
  return who + many + where
}

/** The body: the message on one line (the title already names the author), or only that there is
 *  one when message text is hidden. Not the conversation list's preview: that one stays as it is. */
export function notificationBody(m: Message, me: string, showText: boolean): string {
  if (!showText) return m.mentions.includes(me) ? 'Mentioned you' : 'New message'
  const text = m.content.replace(new RegExp(TOKEN_RE, 'g'), ':$2:').replace(/\s+/g, ' ').trim()
  if (text) {
    // by code point, so an emoji is never cut in half
    const chars = Array.from(text)
    return chars.length > BODY_MAX ? chars.slice(0, BODY_MAX - 1).join('') + '…' : text
  }
  if (m.clip) return 'sent a clip'
  if (m.quote) return 'shared a quote'
  if (m.qoty_card) return 'posted the Quote of the Year card'
  return 'New message'
}

export const NOTIFY_WINDOW_MS = 4000

type Window = { timer: ReturnType<typeof setTimeout>; shown: number; pending: number; newest: ((more: number) => Notice) | null }

/** Keeps a busy conversation from burying the desktop: its first message notifies at once and opens
 *  a [NOTIFY_WINDOW_MS] window. Whatever arrives inside it is counted, and when it ends one
 *  notification (the newest, "and N more") replaces the last and opens the next window. N counts
 *  every message of the run not shown: the OS entry was replaced, so the earlier ones are gone. */
export class NotifyThrottle {
  private windows = new Map<string, Window>()

  constructor(
    private send: (n: Notice) => void,
    private windowMs = NOTIFY_WINDOW_MS,
  ) {}

  /** A new message for conversation `key`; `build(more)` makes its notification. */
  offer(key: string, build: (more: number) => Notice): void {
    const w = this.windows.get(key)
    if (w) {
      w.pending++
      w.newest = build
      return
    }
    this.send(build(0))
    this.open(key, { shown: 1, pending: 0, newest: null })
  }

  /** Drops every window and pending replacement (the window got focus, or you signed out). */
  clear(): void {
    for (const w of this.windows.values()) clearTimeout(w.timer)
    this.windows.clear()
  }

  private open(key: string, w: Omit<Window, 'timer'>) {
    const timer = setTimeout(() => {
      const cur = this.windows.get(key)
      if (cur?.timer !== timer) return
      this.windows.delete(key)
      if (!cur.pending || !cur.newest) return
      this.send(cur.newest(cur.shown + cur.pending - 1))
      this.open(key, { shown: cur.shown + cur.pending, pending: 0, newest: null })
    }, this.windowMs)
    this.windows.set(key, { ...w, timer })
  }
}
