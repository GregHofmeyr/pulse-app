// @vitest-environment jsdom
// Editing a poll (#265): its creator's Edit poll opens the composer filled from the poll, with Save changes; the
// set-up stays as posted; adding an option saves straight away; rewording or removing a voted option asks first,
// naming the votes it clears, and then saves with `clear_votes`. The card says "(edited)".
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message } from '../lib/protocol/Message'
import type { Poll } from '../lib/protocol/Poll'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import PollComposer from './PollComposer.svelte'
import PollCard from './PollCard.svelte'
import { resetState } from '../lib/store.svelte'
import { resetPolls } from '../lib/polls.svelte'
import { flushMicrotasks } from '../lib/testSettle'

const poll = (over: Partial<Poll> = {}): Poll => ({
  question: 'Food?', options: ['Pizza', 'Tacos'], multi: true, anonymous: false, ends_at: null, closed_at: null, closed: false,
  voters: 2, mine: [], results: [{ votes: 2, voters: ['U2', 'U3'] }, { votes: 0, voters: [] }], ...over,
})
const message = (p: Poll): Message => ({
  id: 'M1', channel_id: 'C1', author_id: 'U1', kind: 'normal', content: '📊 Poll: Food?', reply_to_id: null,
  created_at: '2026-11-09T10:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null,
  quote: null, qoty_card: null, clip: null, embeds: [], poll: p,
})

let cmp: ReturnType<typeof mount> | undefined
let closed = 0
function open(p = poll()) {
  const anchor = document.createElement('button')
  document.body.append(anchor)
  cmp = mount(PollComposer, { target: document.body, props: { anchor, channelId: 'C1', editing: message(p), onClose: () => closed++ } })
  flushSync()
}
const pop = () => document.querySelector<HTMLElement>('[role="dialog"][aria-label="Edit poll"]')!
const optionFields = () => [...pop().querySelectorAll<HTMLInputElement>('ol input')]
const button = (text: string) => [...pop().querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent!.trim() === text)
function fill(field: HTMLInputElement, value: string) {
  field.value = value
  field.dispatchEvent(new Event('input'))
  flushSync()
}

beforeEach(() => {
  resetState()
  resetPolls()
  invoke.mockReset()
  invoke.mockResolvedValue(poll())
  closed = 0
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('editing a poll', () => {
  it('opens filled from the poll, with the set-up fixed and Save changes', () => {
    open()
    expect(pop().querySelector<HTMLInputElement>('input[aria-label="Question"]')!.value).toBe('Food?')
    expect(optionFields().map((f) => f.value)).toEqual(['Pizza', 'Tacos'])
    expect(pop().querySelector('input[role="switch"]')).toBeNull()
    expect(pop().textContent).toContain('Multiple choice and the end stay as posted.')
    expect(button('Save changes')).toBeTruthy()
  })

  it('adding an option and changing the question save straight away, keeping each option', async () => {
    open()
    fill(pop().querySelector<HTMLInputElement>('input[aria-label="Question"]')!, 'Dinner?')
    button('Add option')!.click()
    flushSync()
    fill(optionFields()[2], 'Curry')
    button('Save changes')!.click()
    await flushMicrotasks()
    expect(invoke).toHaveBeenCalledWith('edit_poll', { messageId: 'M1', request: {
      question: 'Dinner?', options: [{ text: 'Pizza', from: 0 }, { text: 'Tacos', from: 1 }, { text: 'Curry', from: null }], clear_votes: false,
    } })
    expect(closed).toBe(1)
  })

  it('rewording a voted option asks first, naming the votes, then saves with clear_votes', async () => {
    open()
    fill(optionFields()[0], 'Pizza!')
    button('Save changes')!.click()
    await flushMicrotasks()
    expect(invoke).not.toHaveBeenCalled()
    expect(pop().querySelector('[role="alert"]')!.textContent).toBe('This clears 2 votes (Pizza).')
    button('Clear votes and save')!.click()
    await flushMicrotasks()
    expect(invoke).toHaveBeenCalledWith('edit_poll', { messageId: 'M1', request: {
      question: 'Food?', options: [{ text: 'Pizza!', from: 0 }, { text: 'Tacos', from: 1 }], clear_votes: true,
    } })
  })

  it('removing an option without votes needs no confirm; putting a reword back drops the confirm', async () => {
    open(poll({ options: ['Pizza', 'Tacos', 'Sushi'], results: [{ votes: 2, voters: [] }, { votes: 0, voters: [] }, { votes: 0, voters: [] }] }))
    fill(optionFields()[0], 'Pizza!')
    button('Save changes')!.click()
    await flushMicrotasks()
    expect(button('Clear votes and save')).toBeTruthy()
    fill(optionFields()[0], 'Pizza')
    expect(button('Save changes')).toBeTruthy()
    pop().querySelector<HTMLButtonElement>('button[aria-label="Remove option 2"]')!.click()
    flushSync()
    button('Save changes')!.click()
    await flushMicrotasks()
    expect(invoke).toHaveBeenCalledWith('edit_poll', { messageId: 'M1', request: {
      question: 'Food?', options: [{ text: 'Pizza', from: 0 }, { text: 'Sushi', from: 2 }], clear_votes: false,
    } })
  })

  it('the card says (edited) once its creator changed it', () => {
    const nameOf = () => 'sam'
    cmp = mount(PollCard, { target: document.body, props: { message: message(poll({ edited_at: '2026-11-09T11:00:00.000Z' })), nameOf } })
    flushSync()
    expect(document.querySelector('.poll .status')!.textContent).toContain('(edited)')
  })
})
