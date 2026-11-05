// @vitest-environment jsdom
// Opening a channel or conversation with the pointer puts you in its message box, caret at the end
// of any draft (as Discord does). Not from the keyboard (you're moving through a list), and not
// while a dialog or menu is open.
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))

import TextChannel from './TextChannel.svelte'
import { app, resetState } from '../lib/store.svelte'
import { applyReady, emptyState, markHistory } from '../lib/state'
import { testReady } from '../lib/testReady'
import { offsetOf, serialize } from '../lib/composerDom'

const box = () => document.querySelector<HTMLElement>('[role="textbox"]')!
const caret = () => {
  const s = getSelection()!
  return offsetOf(box(), s.anchorNode!, s.anchorOffset)
}
/** How the channel was picked: a press (pointer) or a key (Enter/arrows in a list). */
const pick = (how: 'pointer' | 'key') => {
  const row = document.createElement('button')
  document.body.append(row)
  row.focus()
  if (how === 'pointer') row.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, pointerType: 'mouse' }))
  else row.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
  row.remove()
}

let c: ReturnType<typeof mount> | undefined
const open = async (channelId: string) => {
  c = mount(TextChannel, { target: document.body, props: { channelId, serverId: channelId === 'D1' ? null : 'S1' } })
  flushSync()
  await tick()
  await tick()
}

beforeEach(() => {
  Element.prototype.scrollTo = () => {}
  resetState()
  app.state = applyReady(emptyState(), testReady([
    { id: 'C1', server_id: 'S1', kind: 'text', name: 'general', position: 0 },
    { id: 'Q1', server_id: 'S1', kind: 'quotes', name: 'quotes', position: 1 },
  ]))
  for (const id of ['C1', 'Q1']) app.state = markHistory(app.state, id, true)
})
afterEach(() => {
  if (c) unmount(c)
  c = undefined
  document.body.innerHTML = ''
})

describe('TextChannel: the message box takes focus when opened', () => {
  it('a channel picked with the pointer focuses its message box', async () => {
    pick('pointer')
    await open('C1')
    expect(document.activeElement).toBe(box())
  })

  it('the caret goes to the end of the saved draft', async () => {
    pick('pointer')
    await open('C1')
    box().replaceChildren(document.createTextNode('half a thought'))
    box().dispatchEvent(new InputEvent('input', { bubbles: true }))
    flushSync()
    unmount(c!)
    c = undefined
    pick('pointer')
    await open('C1')
    expect(serialize(box())).toBe('half a thought')
    expect(document.activeElement).toBe(box())
    expect(caret()).toBe('half a thought'.length)
  })

  it('a channel picked from the keyboard leaves focus in the list', async () => {
    pick('key')
    await open('C1')
    expect(document.activeElement).not.toBe(box())
  })

  it('not while a dialog or menu is open', async () => {
    for (const markup of ['<div role="dialog" aria-modal="true"></div>', '<div role="menu"></div>']) {
      document.body.insertAdjacentHTML('beforeend', markup)
      pick('pointer')
      await open('C1')
      expect(document.activeElement).not.toBe(box())
      unmount(c!)
      c = undefined
      document.body.innerHTML = ''
    }
  })

  it('a quotes channel focuses its new-quote box', async () => {
    pick('pointer')
    await open('Q1')
    expect(document.activeElement).toBe(document.querySelector('.new-quote textarea.qtext'))
  })
})
