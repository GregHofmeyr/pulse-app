// Derived views over AppState. Pure, so they're tested without the UI.
import type { Channel } from './protocol/Channel'
import type { StreamInfo } from './protocol/StreamInfo'
import type { User } from './protocol/User'
import type { Member } from './protocol/Member'
import type { AppState } from './state'
import type { Message } from './protocol/Message'
import type { Mentionable } from './mentions'
import { isHidden } from './blocks'

const byPosition = (a: Channel, b: Channel) => a.position - b.position || (a.name ?? '').localeCompare(b.name ?? '')

export function channelsFor(s: AppState, serverId: string): { text: Channel[]; quotes: Channel[]; clips: Channel[]; voice: Channel[] } {
  const mine = Object.values(s.channels).filter((c) => c.server_id === serverId)
  return {
    text: mine.filter((c) => c.kind === 'text').sort(byPosition),
    quotes: mine.filter((c) => c.kind === 'quotes').sort(byPosition),
    clips: mine.filter((c) => c.kind === 'clips').sort(byPosition),
    voice: mine.filter((c) => c.kind === 'voice').sort(byPosition),
  }
}

export function isMember(s: AppState, serverId: string): boolean {
  return !!s.me && (s.members[serverId] ?? []).some((m) => m.user.id === s.me!.id)
}

/** How someone's name looks by default: their display name (an older server sends none: then the
 *  username). */
export function baseName(u: Pick<User, 'username' | 'display_name'>): string {
  return u.display_name || u.username
}

/** The server nickname someone has in `serverId`, if they're in it and have one. */
export function serverNick(s: AppState, serverId: string | null, userId: string): string | null {
  if (!serverId) return null
  const list = s.members[serverId]
  return list ? (memberIndex(list).get(userId)?.nickname ?? null) : null
}

// A server's members by user id, per member list: the state replaces a list whenever it changes, so an index made
// for one stays right for as long as that list is held (and goes with it).
const memberIndexes = new WeakMap<Member[], Map<string, Member>>()
function memberIndex(list: Member[]): Map<string, Member> {
  let index = memberIndexes.get(list)
  if (!index) {
    index = new Map()
    // the first entry for a person wins, as `find` would pick it
    for (const m of list) if (!index.has(m.user.id)) index.set(m.user.id, m)
    memberIndexes.set(list, index)
  }
  return index
}

/** A server channel as the UI writes it: "#general", but a voice channel bare ("Lounge", #230). */
export function channelLabel(ch: Pick<Channel, 'kind' | 'name'>): string {
  return ch.kind === 'voice' ? (ch.name ?? '') : `#${ch.name ?? ''}`
}

/** THE name shown for a person (#47, #48), used wherever one renders. Inside a server: their
 *  nickname there, else the private nickname you gave them, else their display name. Everywhere
 *  else (`serverId` null): your private nickname for them, else their display name. The username is
 *  never a name once a display name exists; only the profile card shows it. */
export function displayName(s: AppState, serverId: string | null, userId: string): string {
  const nick = serverNick(s, serverId, userId) ?? (userId !== s.me?.id ? s.privateNicks[userId] : undefined)
  if (nick) return nick
  const u = userId === s.me?.id ? s.me : userOf(s, userId)
  return u ? baseName(u) : 'unknown'
}

/** `serverMutedBy` / `serverDeafenedBy`: who in the server did it (#140), absent when not in force. */
export type Occupant = { userId: string; name: string; muted: boolean; deafened: boolean; serverMutedBy?: string; serverDeafenedBy?: string }

/** Who server muted / deafened `userId` in `serverId` (never in a DM or group call). */
export function serverVoiceOf(s: AppState, serverId: string | null, userId: string): { serverMutedBy?: string; serverDeafenedBy?: string } {
  const m = serverId ? s.serverVoice[serverId]?.[userId] : undefined
  return { serverMutedBy: m?.muted_by ?? undefined, serverDeafenedBy: m?.deafened_by ?? undefined }
}

/** Whether `userId` is in one of `serverId`'s voice channels (where a server mute or deafen can be put on them, #144). */
export function inServerVoice(s: AppState, serverId: string, userId: string): boolean {
  return Object.entries(s.voice).some(([c, members]) => s.channels[c]?.server_id === serverId && members.some((m) => m.user_id === userId))
}

/** "Server deafened by sam" / "Server muted by sam" for someone in a call (#140), or null. */
export function serverVoiceTip(s: AppState, serverId: string | null, o: { serverMutedBy?: string; serverDeafenedBy?: string }): string | null {
  if (o.serverDeafenedBy) return `Server deafened by ${displayName(s, serverId, o.serverDeafenedBy)}`
  if (o.serverMutedBy) return `Server muted by ${displayName(s, serverId, o.serverMutedBy)}`
  return null
}

export function voiceOccupants(s: AppState, serverId: string | null, channelId: string): Occupant[] {
  return (s.voice[channelId] ?? []).map((m) => ({
    userId: m.user_id,
    name: displayName(s, serverId, m.user_id),
    muted: m.flags.muted,
    deafened: m.flags.deafened,
    ...serverVoiceOf(s, serverId, m.user_id),
  }))
}

export type SelfVoice = { channelId: string | null; joining: boolean; leaving: string | null }
export type RosterEntry = Occupant & { pending: boolean }

/** Who to show in a voice channel: the server's list, plus you straight away while joining and
 * minus you straight away after leaving. */
export function callRoster(s: AppState, serverId: string | null, channelId: string, me: SelfVoice): RosterEntry[] {
  const myId = s.me?.id
  // someone blocked with Hide completely (#474) has no row, and isn't counted
  let list: RosterEntry[] = voiceOccupants(s, serverId, channelId).filter((o) => !isHidden(s, o.userId)).map((o) => ({ ...o, pending: o.userId === myId && channelId === me.channelId && me.joining }))
  if (channelId === me.leaving && channelId !== me.channelId) list = list.filter((o) => o.userId !== myId)
  if (myId && s.me && channelId === me.channelId && !list.some((o) => o.userId === myId)) {
    list.push({ userId: myId, name: displayName(s, serverId, myId), muted: false, deafened: false, ...serverVoiceOf(s, serverId, myId), pending: me.joining })
  }
  return list
}

export function typingNames(s: AppState, serverId: string | null, channelId: string, now: number): string[] {
  return Object.entries(s.typing[channelId] ?? {})
    .filter(([user, until]) => until > now && user !== s.me?.id && !isHidden(s, user))
    .map(([user]) => displayName(s, serverId, user))
}

/** `prev` when `next` has the same items in the same order, else `next`: keeps a derived list's
 * identity across events that didn't change it, so components fed with it don't re-render. */
export function stableList<T>(prev: T[], next: T[]): T[] {
  return prev.length === next.length && prev.every((v, i) => v === next[i]) ? prev : next
}

/** `stableList` with its memory: each call compares with the list it handed out last time. For a
 * `$derived` whose identity should only change with its items. */
export function keepStable<T>(): (next: T[]) => T[] {
  let prev: T[] = []
  return (next) => (prev = stableList(prev, next))
}

/** When the next typing indicator in `channelId` runs out (ms timestamp), or null if nobody is typing.
 * Your own typing is left out, as in `typingNames`: it's never shown, so its expiry needs no wakeup. */
export function nextTypingExpiry(s: AppState, channelId: string, now: number): number | null {
  const live = Object.entries(s.typing[channelId] ?? {})
    .filter(([user, until]) => until > now && user !== s.me?.id)
    .map(([, until]) => until)
  return live.length ? Math.min(...live) : null
}

const NONE: Record<string, StreamInfo> = {}

/** The live streams in a voice channel, by streamer's user id. */
export function liveIn(s: AppState, channelId: string): Record<string, StreamInfo> {
  return s.streams[channelId] ?? NONE
}

export function isLive(s: AppState, channelId: string, userId: string): boolean {
  return !!s.streams[channelId]?.[userId]
}

/** Whether message `i` continues the previous one's group (same author, within 5 minutes, no inline reply). */
export function groupedWith(list: Message[], i: number): boolean {
  const m = list[i]
  const prev = list[i - 1]
  return !!prev && prev.author_id === m.author_id && prev.kind === 'normal' && !m.reply_to_id
    && new Date(m.created_at).getTime() - new Date(prev.created_at).getTime() < 5 * 60_000
}

/** Whether pending message `i` continues the group before it: the same rule as `groupedWith`, against the confirmed
 *  messages followed by the pending ones, so a sent message doesn't show its own header for a round trip and then
 *  collapse once it's confirmed (#173). */
export function groupedPending(confirmed: Message[], pending: Message[], i: number): boolean {
  const prev = i > 0 ? pending[i - 1] : confirmed[confirmed.length - 1]
  return prev ? groupedWith([prev, pending[i]], 1) : false
}

/** Every name someone can be found by in `serverId`'s views (or outside a server): display name,
 *  server nickname, your private nickname for them, without repeats. Not the username. */
export function namesOf(s: AppState, serverId: string | null, userId: string): string[] {
  const u = userOf(s, userId)
  const all = [u ? baseName(u) : '', serverNick(s, serverId, userId) ?? '', userId !== s.me?.id ? (s.privateNicks[userId] ?? '') : '']
  return all.filter((n, i) => n && all.indexOf(n) === i)
}

/** A server's members as pickers list them (speakers, search's `from:`): the name shown there, and
 *  the username and other names they can be found by. */
export function serverPeople(s: AppState, serverId: string): Mentionable[] {
  return (s.members[serverId] ?? []).map((m) => ({ id: m.user.id, name: displayName(s, serverId, m.user.id), username: m.user.username, alts: namesOf(s, serverId, m.user.id) }))
}

/** Who can be @mentioned in a channel: a DM/group's members, or everyone for a server's channels (never me),
 *  as the name shown there, their username (what a mention is written as) and every name they can be found by. */
export function mentionablesFor(s: AppState, channelId: string): Mentionable[] {
  const ch = s.channels[channelId]
  const serverId = ch?.server_id ?? null
  // Everything the list is made from (each person is in `people`, so their name never needs another lookup). Most
  // events change none of it, so the last list made for the channel is handed out again: the state is replaced on
  // every event, and a server's whole member list would otherwise be rebuilt each time.
  // Of `people`, only who is in it and their user objects (a presence change keeps both, so it keeps the list).
  const deps = [ch, s.dmMembers[channelId], peopleUsers(s.people), s.me, serverId === null ? undefined : s.members[serverId], s.privateNicks]
  const memo = mentionableMemo.get(channelId)
  if (memo && memo.deps.every((d, i) => d === deps[i])) return memo.list
  const ids = ch?.server_id === null ? (s.dmMembers[channelId] ?? []) : Object.keys(s.people)
  const list = ids
    .filter((id) => id !== s.me?.id && s.people[id])
    .map((id) => ({ id, name: displayName(s, serverId, id), username: s.people[id].user.username, alts: namesOf(s, serverId, id) }))
  if (mentionableMemo.size >= MENTIONABLE_MEMO_MAX && !memo) mentionableMemo.clear() // a few open chats' worth
  mentionableMemo.set(channelId, { deps, list, pills: null })
  return list
}

/** The user objects of `people`, in its order: the same array as last time while those are unchanged, so a presence or
 *  activity change (which keeps each person's `user`) doesn't count as a change to who can be mentioned. */
function peopleUsers(people: AppState['people']): User[] {
  if (people === usersOf.people) return usersOf.users
  const users = Object.values(people).map((p) => p.user)
  usersOf.people = people
  usersOf.users = stableList(usersOf.users, users)
  return usersOf.users
}
const usersOf: { people: AppState['people'] | null; users: User[] } = { people: null, users: [] }

// The last mentionables (and pill names) made per channel, with what they were made from.
const MENTIONABLE_MEMO_MAX = 32
const mentionableMemo = new Map<string, { deps: unknown[]; list: Mentionable[]; pills: string[] | null }>()

/** The usernames a message's `@name` pills are made for in a channel: everyone mentionable there, and
 *  you (a mention of you shows your name too, #47; you're only left out of the autocomplete). */
export function pillNamesFor(s: AppState, channelId: string): string[] {
  const people = mentionablesFor(s, channelId)
  const memo = mentionableMemo.get(channelId)!
  if (memo.pills) return memo.pills
  const names = people.map((p) => p.username)
  return (memo.pills = s.me ? [...names, s.me.username] : names)
}

/** A name lookup for one server's views (`state` reads the current state); no author is "Pulse". */
export function nameOfIn(state: () => AppState, serverId: string | null): (userId: string | null) => string {
  return (userId) => (userId ? displayName(state(), serverId, userId) : 'Pulse')
}

/** The freshest copy of a user we hold: people, then any server's members, then me. */
/** Told the id of someone [userOf] can't find (#486: Ready lists only the people you share a server, a DM or a group
 *  with, so the author of an older message may be missing). The store looks them up; nothing is set in tests. */
let unknownPerson: ((id: string) => void) | null = null
export function onUnknownPerson(hook: ((id: string) => void) | null) {
  unknownPerson = hook
}

export function userOf(s: AppState, id: string): User | null {
  if (s.people[id]) return s.people[id].user
  for (const list of Object.values(s.members)) {
    const m = list.find((x) => x.user.id === id)
    if (m) return m.user
  }
  if (s.me?.id === id) return s.me
  // someone you share nothing with now, looked up by id (their profile, once it's in)
  const looked = s.profiles[id]
  if (looked) return looked.user
  unknownPerson?.(id)
  return null
}

/** The user's role colour in a server, if any (always none until the roles push). */
export function roleColorFor(s: AppState, serverId: string | null, userId: string): string | null {
  return serverId ? (s.roleColors[serverId]?.[userId] ?? null) : null
}
