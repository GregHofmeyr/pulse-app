// @vitest-environment jsdom
// The composer's emoji and GIF picker (#461, GIFs from #58): one button opens one picker with Emoji | GIFs tabs where
// GIFs can be sent, each tab with its own search, both the same size, opening on the tab last used. A GIF pick sends it
// at once (what's typed stays) and closes the picker.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (cmd: string, _args?: unknown): Promise<unknown> => {
  if (cmd === 'gif_feed') {
    return { gifs: [{ id: 'abc', title: 'Yes GIF', url: 'https://giphy.com/gifs/yes-abc', width: 480, height: 270,
      preview: 'https://media1.giphy.com/media/abc/200w.webp', preview_width: 200, preview_height: 113, still: 'https://media1.giphy.com/media/abc/200w_s.gif' }], next: null }
  }
  if (cmd === 'gif_favourites') return []
  return undefined
})
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import Composer from './Composer.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import { resetGifFavs } from '../lib/gifs.svelte'
import { serialize } from '../lib/composerDom'
import { flushMicrotasks } from '../lib/testSettle'

let cmp: ReturnType<typeof mount> | undefined
const onSendGif = vi.fn()
function show(extra: Record<string, unknown> = {}) {
  app.state = { ...emptyState(), me: { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } } }
  cmp = mount(Composer, {
    target: document.body,
    props: { placeholder: 'Message', draftKey: `K${Math.random()}`, replyingTo: null, onCancelReply: () => {}, onSend: () => {}, onTyping: () => {}, ...extra },
  })
  flushSync()
}
const pickerBtn = () => document.querySelector<HTMLButtonElement>('button[aria-label="Emoji and GIFs"]')
const gifPicker = () => document.querySelector('[role="dialog"][aria-label="GIF picker"]')
const emojiPicker = () => document.querySelector('[role="dialog"][aria-label="Emoji picker"]')
const tab = (label: string) => [...document.querySelectorAll<HTMLButtonElement>('[role=radiogroup][aria-label="Emoji or GIFs"] [role=radio]')].find((b) => b.textContent === label)

beforeEach(() => {
  resetGifFavs()
  onSendGif.mockClear()
  localStorage.clear()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('Composer emoji and GIF picker (#461)', () => {
  it('one button on the bar: no separate GIF button, and no tabs where GIFs can\'t be sent', () => {
    show()
    expect(pickerBtn()).toBeNull()
    expect(document.querySelector('button[aria-label="GIF"]')).toBeNull()
    document.querySelector<HTMLButtonElement>('button[aria-label="Emoji"]')!.click()
    flushSync()
    expect(emojiPicker()).not.toBeNull()
    expect(tab('GIFs')).toBeUndefined()
    unmount(cmp!)
    document.body.innerHTML = ''
    show({ onSendGif })
    expect(pickerBtn()).not.toBeNull()
    expect(document.querySelector('button[aria-label="GIF"]')).toBeNull()
  })

  it('opens on Emoji the first time; the tabs switch, and search follows the tab', async () => {
    show({ onSendGif })
    pickerBtn()!.click()
    flushSync()
    expect(pickerBtn()!.getAttribute('aria-expanded')).toBe('true')
    expect(emojiPicker()).not.toBeNull()
    expect(tab('Emoji')!.getAttribute('aria-checked')).toBe('true')
    expect(document.activeElement?.getAttribute('aria-label')).toBe('Search emoji')
    tab('GIFs')!.click()
    flushSync()
    await flushMicrotasks(6)
    expect(gifPicker()).not.toBeNull()
    expect(emojiPicker()).toBeNull()
    expect(document.activeElement?.getAttribute('aria-label')).toBe('Search GIPHY')
    pickerBtn()!.click()
    flushSync()
    expect(gifPicker()).toBeNull()
  })

  it('opens on the tab last used', async () => {
    show({ onSendGif })
    pickerBtn()!.click()
    flushSync()
    tab('GIFs')!.click()
    flushSync()
    pickerBtn()!.click()
    flushSync()
    pickerBtn()!.click()
    flushSync()
    await flushMicrotasks(6)
    expect(gifPicker()).not.toBeNull()
    expect(tab('GIFs')!.getAttribute('aria-checked')).toBe('true')
  })

  it('both tabs are the same size, and the tab buttons too', async () => {
    show({ onSendGif })
    pickerBtn()!.click()
    flushSync()
    const group = document.querySelector('[role=radiogroup][aria-label="Emoji or GIFs"]')!
    expect(group.classList.contains('fill')).toBe(true)
    expect(document.querySelector('.emoji-popover')!.classList.contains('has-tabs')).toBe(true)
  })

  it('ArrowRight in the tab row switches tab', async () => {
    show({ onSendGif })
    pickerBtn()!.click()
    flushSync()
    tab('Emoji')!.focus()
    tab('Emoji')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true }))
    flushSync()
    await flushMicrotasks(6)
    expect(gifPicker()).not.toBeNull()
  })

  it('a GIF pick sends it at once, keeps what was typed and closes the picker', async () => {
    show({ onSendGif })
    const box = document.querySelector<HTMLElement>('[role="textbox"]')!
    box.replaceChildren(document.createTextNode('half a thought'))
    box.dispatchEvent(new InputEvent('input', { bubbles: true }))
    flushSync()
    pickerBtn()!.click()
    flushSync()
    tab('GIFs')!.click()
    flushSync()
    await flushMicrotasks(6)
    document.querySelector<HTMLButtonElement>('.pick[aria-label="Yes GIF"]')!.click()
    flushSync()
    expect(onSendGif).toHaveBeenCalledWith({ id: 'abc', title: 'Yes GIF', width: 480, height: 270 }, 'https://giphy.com/gifs/yes-abc')
    expect(gifPicker()).toBeNull()
    expect(serialize(box)).toBe('half a thought')
  })
})
