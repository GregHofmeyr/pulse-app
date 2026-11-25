// Pin to side (#619, spec §4.4): one build kept in the right-hand column while you move between channels, DMs and
// voice. One at a time (pinning another replaces it); it survives a restart (localStorage, each account its own key),
// and each Ready fetches it again. Its copy here is kept current by live events. Deleted, or out of reach (its channel
// left the data: you left the server, it was deleted), it is unpinned at once and the panel says it's gone until
// dismissed. A sign-out lets it go from memory (the next account never sees it), and it shows only for the account it
// belongs to (`pinShown`).
import type { Event } from './protocol/Event'
import type { Message } from './protocol/Message'
import { isNotFound } from './soundboard'
import { app, onAppEvent, onAppReady, onAppReset } from './store.svelte'
import { api } from './tauri'

/** Each account's pin is saved under `KEY.<user id>`. */
const KEY = 'pulse.pinnedBuild'

export type PinnedKey = { channelId: string; messageId: string }

/** `key`: the pinned build (null: none). `message`: its copy once fetched. `gone`: it was deleted or can't be
 *  reached any more (already unpinned in storage; the panel shows a note until Dismiss). `owner`: the account it
 *  belongs to. */
export const pinnedBuild = $state({ key: null as PinnedKey | null, message: null as Message | null, gone: false, owner: null as string | null })

/** Bumped by every pin, unpin, fetch and live change to the copy: an older fetch's answer is dropped. */
let seq = 0

/** Whether the pin is for the account signed in (the panel shows only then). */
export const pinShown = (): boolean => !!pinnedBuild.key && !!pinnedBuild.owner && pinnedBuild.owner === app.state.me?.id

const keyOf = (userId: string) => `${KEY}.${userId}`

function parse(raw: string | null): PinnedKey | null {
  try {
    const v: unknown = JSON.parse(raw ?? 'null')
    if (!v || typeof v !== 'object') return null
    const s = v as Partial<PinnedKey>
    return typeof s.channelId === 'string' && typeof s.messageId === 'string' ? { channelId: s.channelId, messageId: s.messageId } : null
  } catch {
    return null
  }
}

/** The account's saved pin. The single key of before (`{ userId, … }`) moves to its account's own key when that
 *  account signs in; anyone else's is left for them. */
function load(userId: string): PinnedKey | null {
  try {
    const old = localStorage.getItem(KEY)
    if (old !== null) {
      const v = (() => { try { return JSON.parse(old) as { userId?: unknown } } catch { return null } })()
      if (!v || typeof v !== 'object' || v.userId === userId) {
        const k = v ? parse(old) : null
        localStorage.removeItem(KEY)
        if (k && localStorage.getItem(keyOf(userId)) === null) localStorage.setItem(keyOf(userId), JSON.stringify(k))
      }
    }
    return parse(localStorage.getItem(keyOf(userId)))
  } catch {
    return null
  }
}

function save(k: PinnedKey | null) {
  const owner = pinnedBuild.owner
  if (!owner) return
  try {
    if (k) localStorage.setItem(keyOf(owner), JSON.stringify(k))
    else localStorage.removeItem(keyOf(owner))
  } catch {
    /* not kept: it stays pinned until a restart */
  }
}

/** Pin a build (replacing any other) for the account signed in. `message`: its copy when the caller has it; else
 *  it's fetched. */
export function pinBuild(channelId: string, messageId: string, message?: Message): void {
  const me = app.state.me?.id
  if (!me) return
  seq++
  pinnedBuild.owner = me
  pinnedBuild.key = { channelId, messageId }
  pinnedBuild.message = message?.id === messageId ? message : null
  pinnedBuild.gone = false
  save(pinnedBuild.key)
  if (!pinnedBuild.message) void refreshPinnedBuild()
}

/** × or Dismiss: nothing pinned (the owner's saved pin goes; another account's is never touched). */
export function unpinBuild(): void {
  seq++
  if (pinShown() || pinnedBuild.gone) save(null)
  pinnedBuild.key = null
  pinnedBuild.message = null
  pinnedBuild.gone = false
}

/** Whether this build is the pinned one. */
export const isPinned = (messageId: string): boolean => pinShown() && pinnedBuild.key?.messageId === messageId && !pinnedBuild.gone

/** It was deleted or is out of reach: unpinned now (a restart won't bring it back), the note shows until Dismiss. */
function markGone() {
  if (!pinnedBuild.key || pinnedBuild.gone) return
  seq++
  pinnedBuild.gone = true
  save(null)
}

/** A live change to the copy: it wins over any fetch still in flight. */
function setCopy(m: Message) {
  seq++
  pinnedBuild.message = m
}

/** Fetch the pinned build again. "not found" (deleted, or no longer readable: the server says the same) marks it
 *  gone; any other failure (offline) keeps the copy held. */
export async function refreshPinnedBuild(): Promise<void> {
  const k = pinnedBuild.key
  if (!k || pinnedBuild.gone) return
  const mine = ++seq
  try {
    const m = await api.getBuild(k.messageId)
    if (mine !== seq) return
    if (!m?.build || m.deleted) markGone()
    else pinnedBuild.message = m
  } catch (e) {
    if (mine === seq && isNotFound(e)) markGone()
  }
}

/** A live event (after the state applied it): the pinned build's edit, archive or delete, or its channel leaving. */
export function notePinnedBuildEvent(e: Event): void {
  const k = pinnedBuild.key
  if (!k || pinnedBuild.gone) return
  if (e.t === 'MessageUpdated' && !e.d.message.thread_root_id && e.d.message.id === k.messageId) {
    if (e.d.message.deleted || !e.d.message.build) return markGone()
    setCopy(e.d.message)
  } else if (e.t === 'MessageDeleted' && !e.d.thread_root_id && e.d.message_id === k.messageId) {
    return markGone()
  } else if (e.t === 'BuildsArchived' && e.d.message_ids.includes(k.messageId)) {
    const m = pinnedBuild.message
    if (m?.build) setCopy({ ...m, build: { ...m.build, archived_at: e.d.archived_at, archived_by: e.d.archived_by } })
    // no copy yet: the fetch on its way may predate the archive, so it's asked again
    else void refreshPinnedBuild()
  }
  if (!app.state.channels[k.channelId]) markGone()
}

/** A Ready: the account's saved pin (a different account than the one in memory loads its own), fetched again. */
export function notePinnedBuildReady(userId: string): void {
  if (pinnedBuild.owner !== userId) {
    seq++
    pinnedBuild.owner = userId
    pinnedBuild.key = load(userId)
    pinnedBuild.message = null
    pinnedBuild.gone = false
  }
  if (!pinnedBuild.key || pinnedBuild.gone) return
  if (!app.state.channels[pinnedBuild.key.channelId]) return markGone()
  void refreshPinnedBuild()
}

/** Signed out (and tests: as on a fresh start): nothing in memory; what's saved stays the account's. */
export function forgetPinnedBuild(): void {
  seq++
  pinnedBuild.owner = null
  pinnedBuild.key = null
  pinnedBuild.message = null
  pinnedBuild.gone = false
}

onAppReady((r) => notePinnedBuildReady(r.me.id))
onAppEvent(notePinnedBuildEvent)
onAppReset(forgetPinnedBuild)
