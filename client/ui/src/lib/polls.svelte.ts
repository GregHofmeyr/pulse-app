// Polls' client side (#157). A poll's results differ per person (voted or not, its creator), so events carry no
// view: `PollChanged` says one changed and we fetch ours, for a poll this client shows. Votes and closes answer with
// our view directly. Every request takes a ticket, and only an answer newer than the last one shown lands: a slow
// fetch can't put back what a later vote changed.
import type { CreatePollRequest } from './protocol/CreatePollRequest'
import type { Event } from './protocol/Event'
import type { Message } from './protocol/Message'
import type { Poll } from './protocol/Poll'
import { applyEvent, findShown, setPoll } from './state'
import { app, onAppEvent } from './store.svelte'
import { pollCmd } from './tauri'

let tickets = 0
// message id → the ticket of the newest answer shown
const shown = new Map<string, number>()
// message id → a fetch in flight, and whether another is wanted after it
const fetching = new Map<string, Promise<void>>()
const wanted = new Set<string>()

async function run(channelId: string, id: string, ask: () => Promise<Poll>): Promise<Poll> {
  const ticket = ++tickets
  const poll = await ask()
  if (ticket > (shown.get(id) ?? 0)) {
    shown.set(id, ticket)
    app.state = setPoll(app.state, channelId, id, poll)
  }
  return poll
}

/** Fetch our view of a poll and show it. Changes while a fetch is out fold into one more fetch after it. */
export function refreshPoll(channelId: string, id: string): Promise<void> {
  const busy = fetching.get(id)
  if (busy) {
    wanted.add(id)
    return busy
  }
  const go = run(channelId, id, () => pollCmd.get(id))
    .then(() => {}, () => {}) // a poll deleted meanwhile, or the connection down: the next change fetches again
    .finally(() => {
      fetching.delete(id)
      if (wanted.delete(id)) void refreshPoll(channelId, id)
    })
  fetching.set(id, go)
  return go
}

/** Post a poll; it shows at once (its MessageCreated, when it comes, keeps our view). Rejects with the server's
 *  reason. */
export async function postPoll(channelId: string, request: CreatePollRequest): Promise<Message> {
  const ticket = ++tickets
  const m = await pollCmd.create(channelId, request)
  app.state = applyEvent(app.state, { t: 'MessageCreated', d: { message: m, nonce: null } }, Date.now())
  if (m.poll) {
    shown.set(m.id, ticket)
    app.state = setPoll(app.state, channelId, m.id, m.poll)
  }
  return m
}

/** Vote: `options` is your whole set of choices (none takes your vote back). */
export const votePoll = (m: Message, options: number[]): Promise<Poll> => run(m.channel_id, m.id, () => pollCmd.vote(m.id, options))

/** End your poll now. */
export const closePoll = (m: Message): Promise<Poll> => run(m.channel_id, m.id, () => pollCmd.close(m.id))

export function handlePollEvent(e: Event): void {
  if (e.t === 'PollChanged') {
    const { channel_id: c, message_id: id } = e.d
    if (findShown(app.state, c, id)) void refreshPoll(c, id)
  } else if (e.t === 'MessageCreated') {
    // our own poll from another of our sessions: its creator sees the results from the start
    const m = e.d.message
    if (m.poll && m.author_id && m.author_id === app.state.me?.id && !shown.has(m.id)) void refreshPoll(m.channel_id, m.id)
  }
}

onAppEvent(handlePollEvent)

/** For tests: forget everything. */
export function resetPolls(): void {
  tickets = 0
  shown.clear()
  fetching.clear()
  wanted.clear()
}
