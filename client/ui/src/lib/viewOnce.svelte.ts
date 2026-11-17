// View-once files (#311), the client side: who opened a message's view-once files (fetched when one is shown, then
// kept live by ViewOnceOpened), and the one open. The server enforces it: after you open them it signs no other link
// to them for you, so there's nothing to keep here but what to show.
import type { AttachmentUrl } from './protocol/AttachmentUrl'
import type { ViewOnceState } from './protocol/ViewOnceState'
import type { Event } from './protocol/Event'
import { app, onAppEvent, onAppReady } from './store.svelte'
import { api } from './tauri'

export const viewOnce = $state({ states: {} as Record<string, ViewOnceState> })
const asked = new Set<string>()

/** Fetch a message's state once (until the next Ready). */
export function loadViewOnce(messageId: string): void {
  if (asked.has(messageId)) return
  asked.add(messageId)
  api.viewOnceState(messageId).then(
    (st) => (viewOnce.states = { ...viewOnce.states, [messageId]: st }),
    () => asked.delete(messageId),
  )
}

const markOpened = (messageId: string) =>
  (viewOnce.states = { ...viewOnce.states, [messageId]: { ...(viewOnce.states[messageId] ?? { viewers: [] }), opened: true } })

/** The one open: the files' links. Refused (opened before, on another device, say): this throws, and the state is
 *  asked for again so it shows what's true. */
export async function openViewOnce(messageId: string): Promise<AttachmentUrl[]> {
  try {
    const urls = await api.openViewOnce(messageId)
    markOpened(messageId)
    return urls
  } catch (e) {
    asked.delete(messageId)
    loadViewOnce(messageId)
    throw e
  }
}

/** Forget every state (tests, and a new Ready). */
export function resetViewOnce(): void {
  asked.clear()
  viewOnce.states = {}
}

/** A live event: someone (you, or someone you sent view-once files to) opened them. */
export function noteViewOnceEvent(e: Event): void {
  if (e.t !== 'ViewOnceOpened') return
  const me = app.state.me?.id
  const { message_id: id, viewer_id, viewed_at } = e.d
  if (viewer_id === me) return void markOpened(id)
  const st = viewOnce.states[id] ?? { opened: false, viewers: [] }
  if (st.viewers.some((v) => v.user_id === viewer_id)) return
  viewOnce.states = { ...viewOnce.states, [id]: { ...st, viewers: [...st.viewers, { user_id: viewer_id, viewed_at }] } }
}

onAppReady(resetViewOnce)
onAppEvent(noteViewOnceEvent)
