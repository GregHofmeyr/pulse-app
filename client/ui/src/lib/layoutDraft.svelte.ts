// A reorder of a server's channel list (#54) shows at once: the new order is held as a draft, drawn over the
// server's layout (`applyOrder`), until the save answers. Saved: the server's layout takes over, then the draft goes
// (same order, so nothing flashes). Refused (someone moved things first): the draft goes and the list snaps to the
// server's order. A refusal that beats the change it lost to (#198) keeps the draft until that change lands, or for a
// short hold if nothing comes, so the list makes one move to their order instead of snapping back and moving again.
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

/** How long a refused draft waits for the change that beat it (#198) before the list snaps back anyway. */
export const REFUSED_HOLD_MS = 1500
/** serverId -> ends the hold on a refused draft (without dropping it). */
const holds: Record<string, (() => void) | undefined> = {}

const revisionOf = (serverId: string) => app.state.layouts[serverId]?.revision ?? 0

/** The save made on revision `sent` was refused: drop the draft once the layout has moved past `sent` (now, if it
 *  already has) or the hold passes, whichever is first, then `settle`. A later move's draft is never dropped by it
 *  (that move ends the hold, settling it). */
function dropWhenTheirsLands(serverId: string, sent: number, settle: (() => void)[]): void {
  holds[serverId]?.()
  if (revisionOf(serverId) !== sent) {
    dropDraft(serverId)
    for (const f of settle) f()
    return
  }
  const held = drafts.byServer[serverId]
  const end = () => {
    clearTimeout(timer)
    stop()
    if (holds[serverId] === end) delete holds[serverId]
    for (const f of settle.splice(0)) f()
  }
  const drop = () => {
    if (drafts.byServer[serverId] === held) dropDraft(serverId)
    end()
  }
  const timer = setTimeout(drop, REFUSED_HOLD_MS)
  const stop = $effect.root(() => {
    $effect(() => {
      // not from inside this effect: dropping ends the root it runs in
      if (revisionOf(serverId) !== sent) queueMicrotask(drop)
    })
  })
  holds[serverId] = end
}

function dropDraft(serverId: string): void {
  const { [serverId]: _gone, ...rest } = drafts.byServer
  drafts.byServer = rest
}

/** Save `order` as `serverId`'s list. `basedOn`: the layout revision it was made from (a drag that started before
 *  the list last changed); without it, the revision held (or, after a save of ours in flight, the one that save
 *  returns). Settles once its order is saved, refused (and the list has left the draft), or replaced by a later one
 *  that was. */
export function saveOrder(serverId: string, order: Order, basedOn?: number): Promise<void> {
  holds[serverId]?.()
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
  for (const end of Object.values(holds)) end?.()
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
      for (const f of w.done) f()
    } else {
      const dropped = waiting[serverId]
      delete waiting[serverId]
      ui.notice = errorText(answer.refused)
      landed = null
      // they settle once the list has moved off the draft (a keyboard move puts focus back then)
      dropWhenTheirsLands(serverId, revision, [...w.done, ...(dropped?.done ?? [])])
    }
  }
  sending[serverId] = false
}
