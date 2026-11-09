// @vitest-environment jsdom
// Polls from the message box (#157): `/poll` opens the poll composer, `/poll Question? a, b` opens it filled in
// (never posting straight away), and the composer's poll button opens and closes it.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fakeMatchMedia, FakeIntersectionObserver } from '../lib/testMotion'

const invoke = vi.fn(async (cmd: string, _args?: unknown): Promise<unknown> => {
  if (cmd === 'list_messages') return []
  return undefined
})
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))
vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)

import TextChannel from './TextChannel.svelte'
import { forgetPollDrafts } from './PollComposer.svelte'
import { app, resetState } from '../lib/store.svelte'
import { applyReady, emptyState } from '../lib/state'
import { testReady } from '../lib/testReady'

let cmp: ReturnType<typeof mount> | undefined
function show() {
  app.state = applyReady(emptyState(), testReady([{ id: 'C1', server_id: 'S1', kind: 'text', name: 'general', position: 0 }]))
  cmp = mount(TextChannel, { target: document.body, props: { channelId: 'C1', serverId: 'S1' } })
  flushSync()
  return document.querySelector<HTMLElement>('[role="textbox"]')!
}
function send(box: HTMLElement, text: string) {
  box.replaceChildren(document.createTextNode(text))
  box.focus()
  const r = document.createRange()
  r.setStart(box.firstChild!, text.length)
  getSelection()!.removeAllRanges()
  getSelection()!.addRange(r)
  box.dispatchEvent(new InputEvent('input', { bubbles: true }))
  flushSync()
  box.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }))
  flushSync()
}
const composer = () => document.querySelector<HTMLElement>('[role="dialog"][aria-label="New poll"]')

beforeEach(() => {
  Element.prototype.scrollTo = () => {} // jsdom has none
  resetState()
  forgetPollDrafts()
  invoke.mockClear()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('/poll in the message box', () => {
  it('on its own opens an empty poll composer and sends nothing', () => {
    send(show(), '/poll')
    expect(composer()).not.toBeNull()
    expect(invoke).not.toHaveBeenCalledWith('send_message', expect.anything())
    expect(invoke).not.toHaveBeenCalledWith('create_poll', expect.anything())
  })

  it('with a question and options opens it filled in, without posting', () => {
    send(show(), '/poll Where to eat? Pizza, Sushi')
    expect(composer()!.querySelector<HTMLInputElement>('input[aria-label="Question"]')!.value).toBe('Where to eat?')
    expect([...composer()!.querySelectorAll<HTMLInputElement>('ol input')].map((f) => f.value)).toEqual(['Pizza', 'Sushi'])
    expect(invoke).not.toHaveBeenCalledWith('create_poll', expect.anything())
  })

  it('the poll button by the wheel button opens and closes it', () => {
    show()
    const btn = document.querySelector<HTMLButtonElement>('button[aria-label="Poll"]')!
    btn.click()
    flushSync()
    expect(composer()).not.toBeNull()
    expect(btn.getAttribute('aria-expanded')).toBe('true')
    btn.click()
    flushSync()
    expect(composer()).toBeNull()
  })
})
