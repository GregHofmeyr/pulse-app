// @vitest-environment jsdom
// The composer's GIF button (#58): next to the emoji button where GIFs can be sent, it opens the GIF picker, and a
// pick sends the GIF at once (what's typed stays) and closes the picker.
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
const gifBtn = () => document.querySelector<HTMLButtonElement>('button[aria-label="GIF"]')
const picker = () => document.querySelector('[role="dialog"][aria-label="GIF picker"]')
const flush = async () => { for (let i = 0; i < 6; i++) await Promise.resolve(); flushSync() }

beforeEach(() => {
  resetGifFavs()
  onSendGif.mockClear()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('Composer GIF button', () => {
  it('shows only where GIFs can be sent', () => {
    show()
    expect(gifBtn()).toBeNull()
    unmount(cmp!)
    show({ onSendGif })
    expect(gifBtn()).not.toBeNull()
    expect(document.querySelector('.wrap')!.classList.contains('has-gif')).toBe(true)
  })

  it('toggles the GIF picker', () => {
    show({ onSendGif })
    gifBtn()!.click()
    flushSync()
    expect(picker()).not.toBeNull()
    expect(gifBtn()!.getAttribute('aria-expanded')).toBe('true')
    gifBtn()!.click()
    flushSync()
    expect(picker()).toBeNull()
  })

  it('a pick sends the GIF at once, keeps what was typed and closes the picker', async () => {
    show({ onSendGif })
    const box = document.querySelector<HTMLElement>('[role="textbox"]')!
    box.replaceChildren(document.createTextNode('half a thought'))
    box.dispatchEvent(new InputEvent('input', { bubbles: true }))
    flushSync()
    gifBtn()!.click()
    flushSync()
    await flush()
    document.querySelector<HTMLButtonElement>('.pick[aria-label="Yes GIF"]')!.click()
    flushSync()
    expect(onSendGif).toHaveBeenCalledWith({ id: 'abc', title: 'Yes GIF', width: 480, height: 270 }, 'https://giphy.com/gifs/yes-abc')
    expect(picker()).toBeNull()
    expect(serialize(box)).toBe('half a thought')
  })
})
