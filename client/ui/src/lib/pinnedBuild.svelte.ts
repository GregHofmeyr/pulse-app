// Pin to side (#619, spec §4.4): one build kept in the right-hand column while you move between channels, DMs and
// voice. One at a time (pinning another replaces it); it survives a restart (localStorage, per account), and each
// Ready fetches it again. Its copy here is kept current by live events. Deleted, or out of reach (its channel left
// the data: you left the server, it was deleted), it is unpinned at once and the panel says it's gone until dismissed.
import type { Event } from './protocol/Event'
import type { Message } from './protocol/Message'
import { isNotFound } from './soundboard'
import { app, onAppEvent, onAppReady } from './store.svelte'
import { api } from './tauri'

const KEY = 'pulse.pinnedBuild'

export type PinnedKey = { channelId: string; messageId: string }

/** `key`: the pinned build (null: none). `message`: its copy once fetched. `gone`: it was deleted or can't be
 *  reached any more (already unpinned in storage; the panel shows a note until Dismiss). */
export const pinnedBuild = $state({ key: null as PinnedKey | null, message: null as Message | null, gone: false })

/** The account the pin in memory belongs to (a Ready for another loads theirs). */
let owner: string | null = null
/** Bumped by every pin, unpin and fetch: an older fetch's answer is dropped. */
let seq = 0

type Saved = PinnedKey & { userId: string }

function load(userId: string): PinnedKey | null {
  try {
    const v: unknown = JSON.parse(localStorage.getItem(KEY) ?? 'null')
    if (!v || typeof v !== 'object') return null
    const s = v as Partial<Saved>
    if (s.userId !== userId || typeof s.channelId !== 'string' || typeof s.messageId !== 'string') return null
    return { channelId: s.channelId, messageId: s.messageId }
  } catch {
    return null
  }
}

function save(k: PinnedKey | null) {
  try {
    if (k && owner) localStorage.setItem(KEY, JSON.stringify({ userId: owner, ...k } satisfies Saved))
    else localStorage.removeItem(KEY)
  } catch {
    /* not kept: it stays pinned until a restart */
  }
}

/** Pin a build (replacing any other). `message`: its copy when the caller has it; else it's fetched. */
export function pinBuild(channelId: string, messageId: string, message?: Message): void {
  seq++
  owner = app.state.me?.id ?? owner
  pinnedBuild.key = { channelId, messageId }
  pinnedBuild.message = message?.id === messageId ? message : null
  pinnedBuild.gone = false
  save(pinnedBuild.key)
  if (!pinnedBuild.message) void refreshPinnedBuild()
}

/** × or Dismiss: nothing pinned. */
export function unpinBuild(): void {
  seq++
  pinnedBuild.key = null
  pinnedBuild.message = null
  pinnedBuild.gone = false
  save(null)
}

/** Whether this build is the pinned one. */
export const isPinned = (messageId: string): boolean => pinnedBuild.key?.messageId === messageId && !pinnedBuild.gone

/** It was deleted or is out of reach: unpinned now (a restart won't bring it back), the note shows until Dismiss. */
function markGone() {
  if (!pinnedBuild.key || pinnedBuild.gone) return
  seq++
  pinnedBuild.gone = true
  save(null)
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
    pinnedBuild.message = e.d.message
  } else if (e.t === 'MessageDeleted' && !e.d.thread_root_id && e.d.message_id === k.messageId) {
    return markGone()
  } else if (e.t === 'BuildsArchived' && e.d.message_ids.includes(k.messageId)) {
    const m = pinnedBuild.message
    if (m?.build) pinnedBuild.message = { ...m, build: { ...m.build, archived_at: e.d.archived_at, archived_by: e.d.archived_by } }
  }
  if (!app.state.channels[k.channelId]) markGone()
}

/** A Ready: the account's saved pin (a different account than the one in memory loads its own), fetched again. */
export function notePinnedBuildReady(userId: string): void {
  if (owner !== userId) {
    seq++
    owner = userId
    pinnedBuild.key = load(userId)
    pinnedBuild.message = null
    pinnedBuild.gone = false
  }
  if (!pinnedBuild.key || pinnedBuild.gone) return
  if (!app.state.channels[pinnedBuild.key.channelId]) return markGone()
  void refreshPinnedBuild()
}

/** Tests: as on a fresh start (nothing in memory; storage is the test's). */
export function forgetPinnedBuild(): void {
  seq++
  owner = null
  pinnedBuild.key = null
  pinnedBuild.message = null
  pinnedBuild.gone = false
}

onAppReady((r) => notePinnedBuildReady(r.me.id))
onAppEvent(notePinnedBuildEvent)
