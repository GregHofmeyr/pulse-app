// @vitest-environment jsdom
// The poll composer (#157): two empty rows to start, + Add option up to ten, remove past two, the switches and Ends
// in, Post poll only once it's a poll, a refusal shown inline, and `/poll`'s prefill.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

let refuse: string | null = null
const created = { id: 'M1', channel_id: 'C1', author_id: 'U1', kind: 'normal', content: '📊 Poll: Q', reply_to_id: null,
  created_at: '2026-11-09T10:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null,
  quote: null, qoty_card: null, clip: null, embeds: [],
  poll: { question: 'Q', options: ['a', 'b'], multi: false, anonymous: false, ends_at: null, closed_at: null, closed: false, voters: 0, mine: [], results: [] } }
const invoke = vi.fn(async (cmd: string, _args?: unknown): Promise<unknown> => {
  if (cmd === 'create_poll') {
    if (refuse) throw refuse
    return created
  }
  return undefined
})
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import PollComposer, { forgetPollDrafts } from './PollComposer.svelte'
import { app, resetState } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import { resetPolls } from '../lib/polls.svelte'

let cmp: ReturnType<typeof mount> | undefined
let closed = 0
function open(prefill: { question: string; options: string[] } | null = null) {
  const anchor = document.createElement('button')
  document.body.append(anchor)
  cmp = mount(PollComposer, { target: document.body, props: { anchor, channelId: 'C1', prefill, onClose: () => closed++ } })
  flushSync()
}
const pop = () => document.querySelector<HTMLElement>('[role="dialog"][aria-label="New poll"]')!
const optionFields = () => [...pop().querySelectorAll<HTMLInputElement>('ol input')]
const button = (text: string) => [...pop().querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent!.trim() === text)
const switchFor = (label: string) => [...pop().querySelectorAll('label')].find((l) => l.textContent!.trim() === label)!.querySelector<HTMLInputElement>('input')!
function fill(field: HTMLInputElement, value: string) {
  field.value = value
  field.dispatchEvent(new Event('input'))
  flushSync()
}
const flush = async () => { for (let i = 0; i < 5; i++) await Promise.resolve(); flushSync() }

beforeEach(() => {
  resetState()
  resetPolls()
  forgetPollDrafts()
  invoke.mockClear()
  refuse = null
  closed = 0
  app.state = { ...emptyState(), messages: { C1: [] } }
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('PollComposer', () => {
  it('starts with a question, two empty options, the switches off and 1 day', () => {
    open()
    expect(pop().querySelector<HTMLInputElement>('input[aria-label="Question"]')!.value).toBe('')
    expect(optionFields().map((f) => f.value)).toEqual(['', ''])
    expect(switchFor('Multiple choice').checked).toBe(false)
    expect(switchFor('Anonymous').checked).toBe(false)
    expect(pop().querySelector('[role="radio"][aria-checked="true"]')!.textContent).toBe('1 day')
    expect(pop().querySelector('button[aria-label^="Remove option"]')).toBeNull()
    expect(button('Post poll')!.disabled).toBe(true)
  })

  it('adds options up to ten and removes them down to two', () => {
    open()
    for (let i = 0; i < 8; i++) button('Add option')!.click()
    flushSync()
    expect(optionFields()).toHaveLength(10)
    expect(button('Add option')).toBeUndefined()
    pop().querySelector<HTMLButtonElement>('button[aria-label="Remove option 3"]')!.click()
    flushSync()
    expect(optionFields()).toHaveLength(9)
    expect(button('Add option')).toBeDefined()
    for (let i = 0; i < 7; i++) pop().querySelector<HTMLButtonElement>('button[aria-label="Remove option 1"]')!.click()
    flushSync()
    expect(optionFields()).toHaveLength(2)
    expect(pop().querySelector('button[aria-label^="Remove option"]')).toBeNull()
  })

  it('posts only a question with two different options, with what was chosen', async () => {
    open()
    fill(pop().querySelector<HTMLInputElement>('input[aria-label="Question"]')!, ' Where to eat? ')
    fill(optionFields()[0], 'Pizza')
    expect(button('Post poll')!.disabled).toBe(true)
    fill(optionFields()[1], 'pizza')
    expect(button('Post poll')!.disabled).toBe(true)
    fill(optionFields()[1], 'Sushi')
    expect(button('Post poll')!.disabled).toBe(false)
    switchFor('Anonymous').click()
    ;[...pop().querySelectorAll<HTMLButtonElement>('[role="radio"]')].find((b) => b.textContent === '1 week')!.click()
    flushSync()
    button('Post poll')!.click()
    await flush()
    expect(invoke).toHaveBeenCalledWith('create_poll', {
      channelId: 'C1',
      request: { question: 'Where to eat?', options: ['Pizza', 'Sushi'], multi: false, anonymous: true, ends_in: 'week' },
    })
    expect(closed).toBe(1)
    expect(app.state.messages.C1.map((m) => m.id)).toEqual(['M1'])
  })

  it('shows a refusal and stays open', async () => {
    refuse = 'Too many requests'
    open({ question: 'Q?', options: ['a', 'b'] })
    button('Post poll')!.click()
    await flush()
    expect(pop().querySelector('[role="alert"]')!.textContent).toBe('Too many requests')
    expect(closed).toBe(0)
  })

  it('/poll fills it in; one option still gets a second row', () => {
    open({ question: 'Games?', options: ['Valheim'] })
    expect(pop().querySelector<HTMLInputElement>('input[aria-label="Question"]')!.value).toBe('Games?')
    expect(optionFields().map((f) => f.value)).toEqual(['Valheim', ''])
  })

  it('keeps an unposted poll for the chat', () => {
    open()
    fill(pop().querySelector<HTMLInputElement>('input[aria-label="Question"]')!, 'Later?')
    unmount(cmp!)
    cmp = undefined
    document.body.innerHTML = ''
    open()
    expect(pop().querySelector<HTMLInputElement>('input[aria-label="Question"]')!.value).toBe('Later?')
  })
})
