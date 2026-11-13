// Pinned messages (#178): the lengths a pin can have, and how a pin reads (the marker's tip, the system line, the
// popover's "time left").
import type { ChannelKind } from './protocol/ChannelKind'
import type { Message } from './protocol/Message'
import type { Pin } from './protocol/Pin'
import type { PinDuration } from './protocol/PinDuration'

/** Whether a chat of this kind takes pins: every kind, quotes and clips channels too (#257, their quotes and clips). */
export function takesPins(kind: ChannelKind | undefined): boolean {
  return kind !== undefined
}

/** Whether a plain message (MessageItem) offers Pin message here: in chats, not quotes or clips channels, whose
 *  posts pin from QuoteItem's and the clip views' own menus, and whose plain messages are thread replies (never
 *  pinned). */
export function pinsPlainMessages(kind: ChannelKind | undefined): boolean {
  return kind === 'text' || kind === 'voice' || kind === 'dm' || kind === 'group'
}

/** What a pin's line calls the pinned post: "a quote" in a quotes channel, else "a message" (#257; a clips
 *  channel's pins post no line). */
export function pinnedNoun(kind: ChannelKind | undefined): string {
  return kind === 'quotes' ? 'a quote' : 'a message'
}

/** The pins popover's title, and its header button's name: "Pinned quotes", "Pinned clips", else "Pinned messages". */
export function pinsTitle(kind: ChannelKind | undefined): string {
  return kind === 'quotes' ? 'Pinned quotes' : kind === 'clips' ? 'Pinned clips' : 'Pinned messages'
}

/** The Pin message flyout's choices, in order. */
export const PIN_FOR: readonly { label: string; duration: PinDuration }[] = [
  { label: '1 hour', duration: 'hour' },
  { label: '1 day', duration: 'day' },
  { label: '1 week', duration: 'week' },
  { label: 'Forever', duration: 'forever' },
]

const plural = (n: number, unit: string) => `${n} ${unit}${n === 1 ? '' : 's'}`

/** "2 days left", in the largest whole unit (at least "1 minute left"). */
export function timeLeft(expiresAt: string, now: number): string {
  const ms = Math.max(0, Date.parse(expiresAt) - now)
  const minutes = Math.max(1, Math.floor(ms / 60_000))
  if (minutes < 60) return `${plural(minutes, 'minute')} left`
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return `${plural(hours, 'hour')} left`
  return `${plural(Math.floor(hours / 24), 'day')} left`
}

/** The marker's tooltip: "Pinned by alex · 2 days left", or "Pinned by alex" for a Forever pin. */
export function pinTip(byName: string, expiresAt: string | null, now: number): string {
  return expiresAt ? `Pinned by ${byName} · ${timeLeft(expiresAt, now)}` : `Pinned by ${byName}`
}

/** How long the system line says it was pinned for: "for 1 day", or nothing for Forever. */
export function pinFor(duration: PinDuration): string {
  const label = PIN_FOR.find((p) => p.duration === duration)?.label
  return duration === 'forever' || !label ? '' : `for ${label}`
}

/** The message's pin, unless it has run out (the sweep's event may not have come yet) or the message is gone. */
export function livePin(m: Message, now: number): Pin | null {
  const p = m.pin
  if (!p || m.deleted) return null
  return p.expires_at && Date.parse(p.expires_at) <= now ? null : p
}
