// @vitest-environment jsdom
// A voice channel's chat as the panel beside its stage (#226): the text channel view with a Chat header and a close
// button; it takes the focus only when a click asked for it (#202), and marks nothing read while it's hidden.
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (..._a: unknown[]): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...a) }))

import TextChannel from './TextChannel.svelte'
import { app, resetState } from '../lib/store.svelte'
import { addHistory, applyReady, emptyState, markHistory } from '../lib/state'
import { testReady } from '../lib/testReady'
import { windowFocus } from '../lib/windowFocus.svelte'
import { voice } from '../lib/voice.svelte'
import { reloadVoiceChat, setVoiceChat } from '../lib/voiceChat.svelte'
import type { Message } from '../lib/protocol/Message'

const box = () => document.querySelector<HTMLElement>('[role="textbox"]')
/** A press, as a click on the chat icon makes. */
const press = () => document.body.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, pointerType: 'mouse' }))

const msg = (id: string): Message => ({
  id, channel_id: 'V1', author_id: 'U2', kind: 'normal', content: 'hi', reply_to_id: null, created_at: '2026-11-06T10:00:00Z',
  edited_at: null, deleted: false, reactions: [], mentions: [], embeds: [],
} as unknown as Message)

let c: ReturnType<typeof mount> | undefined
const onClose = vi.fn()
const open = async (shown = true) => {
  c = mount(TextChannel, { target: document.body, props: { channelId: 'V1', serverId: 'S1', panel: { onClose, shown } } })
  flushSync()
  await tick()
  await tick()
}

beforeEach(() => {
  Element.prototype.scrollTo = () => {}
  localStorage.clear()
  reloadVoiceChat()
  resetState()
  invoke.mockClear()
  onClose.mockClear()
  app.state = applyReady(emptyState(), testReady([{ id: 'V1', server_id: 'S1', kind: 'voice', name: 'Lounge', position: 0 }]))
  app.state = markHistory(app.state, 'V1', true)
})
afterEach(() => {
  if (c) unmount(c)
  c = undefined
  document.body.innerHTML = ''
})

describe('TextChannel as the voice chat panel (#226)', () => {
  it('a Chat header with a close button, no search box, the message box named for the channel', async () => {
    await open()
    expect(document.querySelector('.head .title')?.textContent).toBe('Chat')
    expect(document.querySelector('.search')).toBeNull()
    expect(document.querySelector('[aria-label="Search"]')).toBeNull()
    document.querySelector<HTMLButtonElement>('[aria-label="Close chat"]')!.click()
    expect(onClose).toHaveBeenCalledOnce()
    expect(document.body.innerHTML).toContain('Message Lounge')
    expect(document.body.innerHTML).not.toContain('Message #Lounge')
  })

  it("mounting it (the stage shown with the chat left open) doesn't take the focus", async () => {
    press()
    await open()
    expect(document.activeElement).not.toBe(box())
  })

  it('a click on the chat icon or toggle does, also when it was already open', async () => {
    press()
    setVoiceChat(true, true)
    await open()
    expect(document.activeElement).toBe(box())
    ;(document.activeElement as HTMLElement).blur()
    press()
    setVoiceChat(true, true)
    flushSync()
    await tick()
    await tick()
    expect(document.activeElement).toBe(box())
  })

  it('marks read only while it shows', async () => {
    vi.useFakeTimers()
    try {
      windowFocus.focused = true
      app.state = addHistory(app.state, 'V1', [msg('M1')])
      await open(false)
      vi.advanceTimersByTime(1000)
      expect(invoke.mock.calls.filter((x) => x[0] === 'mark_read')).toEqual([])
      unmount(c!)
      await open(true)
      vi.advanceTimersByTime(1000)
      expect(invoke.mock.calls.filter((x) => x[0] === 'mark_read').length).toBe(1)
    } finally {
      vi.useRealTimers()
    }
  })
})

describe('Share to the stage? in the voice chat (#288)', () => {
  const LINK = 'https://open.spotify.com/track/4uLU6hMCjMI75M1A2tKUQ8'
  const typeIn = (t: string) => {
    const b = box()!
    b.replaceChildren(document.createTextNode(t))
    b.dispatchEvent(new InputEvent('input', { bubbles: true }))
    flushSync()
  }
  afterEach(() => {
    voice.channelId = null
    voice.connection = 'disconnected'
  })

  it('offers it while you are in the call, and the chip shares the link to this channel\'s stage', async () => {
    voice.channelId = 'V1'
    voice.connection = 'connected'
    await open()
    typeIn(`listen ${LINK}`)
    const btn = [...document.querySelectorAll<HTMLButtonElement>('.stage-nudge button')].find((b) => b.textContent?.trim() === 'Share to the stage?')!
    btn.click()
    await tick()
    expect(invoke).toHaveBeenCalledWith('share_to_stage', { channelId: 'V1', url: LINK })
  })

  it('not when you are not in this call', async () => {
    voice.channelId = 'V2'
    voice.connection = 'connected'
    await open()
    typeIn(`listen ${LINK}`)
    expect(document.querySelector('.stage-nudge')).toBeNull()
  })
})
