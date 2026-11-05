// A reorder of a server's channel list (#54) shows at once: the new order is held as a draft, drawn over the
// server's layout (`applyOrder`), until the save answers. Saved: the server's layout takes over, then the draft goes
// (same order, so nothing flashes). Refused (someone moved things first): the draft goes and the list snaps to the
// server's order.
//
// Saves to one server go one at a time: a move made while one is in flight shows at once (the draft) and waits; only
// the latest waiting order is kept. When the save in flight lands, the waiting one goes on the revision it returned
// (so quick moves aren't refused by your own previous one); when it's refused, the waiting one is dropped too.
import { applyLayout } from './state'
import { toRequest, type Order } from './layout'
import type { ServerLayout } from './protocol/ServerLayout'
import { api, errorText } from './tauri'
import { app, ui } from './store.svelte'

/** serverId -> the order being saved (or waiting to be). */
export const drafts = $state({ byServer: {} as Record<string, Order> })

/** The order waiting for the save in flight; `done` settles every save it replaced too. */
type Waiting = { order: Order; basedOn: number | undefined; done: (() => void)[] }
const waiting: Record<string, Waiting | undefined> = {}
const sending: Record<string, boolean> = {}

function dropDraft(serverId: string): void {
  const { [serverId]: _gone, ...rest } = drafts.byServer
  drafts.byServer = rest
}

/** Save `order` as `serverId`'s list. `basedOn`: the layout revision it was made from (a drag that started before
 *  the list last changed); without it, the revision held (or, after a save of ours in flight, the one that save
 *  returns). Settles once its order is saved, refused, or replaced by a later one that was. */
export function saveOrder(serverId: string, order: Order, basedOn?: number): Promise<void> {
  drafts.byServer = { ...drafts.byServer, [serverId]: order }
  return new Promise((resolve) => {
    const before = waiting[serverId]
    waiting[serverId] = { order, basedOn, done: [...(before?.done ?? []), resolve] }
    if (!sending[serverId]) void send(serverId)
  })
}

/** Bumped by `resetSaves`: a save answered after it changes nothing. */
let generation = 0

/** Forget every draft and save in flight (tests). */
export function resetSaves(): void {
  generation++
  for (const k of Object.keys(waiting)) delete waiting[k]
  for (const k of Object.keys(sending)) delete sending[k]
  drafts.byServer = {}
}

async function send(serverId: string): Promise<void> {
  const gen = generation
  sending[serverId] = true
  /** The last save of ours that landed in this run: what it was based on, and the revision it made. */
  let landed: { base: number; revision: number } | null = null
  for (let w = waiting[serverId]; w; w = waiting[serverId]) {
    delete waiting[serverId]
    const held = app.state.layouts[serverId]?.revision ?? 0
    // made from the list as our last save left it: on the revision that save made; made from older: as it was
    const ours = landed && (w.basedOn === undefined || w.basedOn === landed.base || w.basedOn === landed.revision)
    const revision: number = landed && ours ? landed.revision : (w.basedOn ?? held)
    let answer: { saved: ServerLayout } | { refused: unknown }
    try {
      answer = { saved: await api.saveLayout(serverId, toRequest(w.order, revision)) }
    } catch (e) {
      answer = { refused: e }
    }
    if (gen !== generation) return
    if ('saved' in answer) {
      app.state = applyLayout(app.state, answer.saved)
      landed = { base: revision, revision: answer.saved.revision }
      if (!waiting[serverId]) dropDraft(serverId)
    } else {
      const dropped = waiting[serverId]
      delete waiting[serverId]
      dropDraft(serverId)
      ui.notice = errorText(answer.refused)
      landed = null
      for (const f of dropped?.done ?? []) f()
    }
    for (const f of w.done) f()
  }
  sending[serverId] = false
}
