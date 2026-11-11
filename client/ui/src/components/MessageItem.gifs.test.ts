// @vitest-environment jsdom
// GIFs in chat (#58): a picked GIF and a pasted GIPHY link look the same (drawn from GIPHY's media links for the id),
// a message that is only the link shows only the GIF, a GIPHY preview card never shows, reduced motion shows the
// still until hovered or clicked, and the star keeps a GIF in your favourites.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message } from '../lib/protocol/Message'
import type { LinkEmbed } from '../lib/protocol/LinkEmbed'

const invoke = vi.fn(async (cmd: string, _args?: unknown): Promise<unknown> => (cmd === 'gif_favourites' ? [] : undefined))
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import MessageItem from './MessageItem.svelte'
import { motion } from '../lib/animate.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import { gifFavs, resetGifFavs } from '../lib/gifs.svelte'
import { previews } from '../lib/linkPreviews.svelte'

const ID = '3o7TKSjRrfIPjeiVyM'
const LINK = `https://giphy.com/gifs/excited-yes-${ID}`
const ANIM = `https://media.giphy.com/media/${ID}/200.webp`
const STILL = `https://media.giphy.com/media/${ID}/200_s.gif`
const msg = (content: string, over: Partial<Message> = {}): Message => ({
  id: 'M1', channel_id: 'C1', author_id: 'U2', kind: 'normal', content, reply_to_id: null, created_at: '2026-11-11T09:00:00.000Z',
  edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null, quote: null, qoty_card: null, clip: null, embeds: [], ...over,
})
const picked = (over: Partial<Message> = {}) => msg(LINK, { gif: { id: ID, title: 'Excited Yes GIF', width: 480, height: 270 }, ...over })

let cmp: ReturnType<typeof mount> | undefined
function show(m: Message) {
  app.state = {
    ...emptyState(),
    me: { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } },
    channels: { C1: { id: 'C1', server_id: 'S1', kind: 'text', name: 'general', position: 0 } },
    messages: { C1: [m] },
  }
  cmp = mount(MessageItem, { target: document.body, props: {
    message: m, serverId: null, name: 'sam', nameOf: () => 'sam', replyTo: null,
    grouped: false, mine: false, onReply: () => {}, onEdit: async () => {}, onDelete: async () => {}, meId: 'U1',
  } })
  flushSync()
}
const gifs = () => [...document.querySelectorAll<HTMLElement>('.gif')]
const img = (g = gifs()[0]) => g.querySelector('img')!
const content = () => document.querySelector('.content')
const flush = async () => { for (let i = 0; i < 6; i++) await Promise.resolve(); flushSync() }

beforeEach(() => {
  motion.reducedMotion = false
  resetGifFavs()
  invoke.mockClear()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('GIFs in chat', () => {
  it('a picked GIF shows alone, playing, sized by its shape, its title the alt text', () => {
    show(picked())
    expect(content()).toBeNull() // only the GIF: its link is for older clients
    expect(gifs()).toHaveLength(1)
    expect(img().getAttribute('alt')).toBe('Excited Yes GIF')
    expect(img().getAttribute('src')).toBe(ANIM)
    expect(gifs()[0].style.width).toBe('300px')
    expect(gifs()[0].style.height).toBe('169px')
  })

  it('a pasted GIPHY link renders the same way, titled from its page link', () => {
    show(msg(LINK))
    expect(content()).toBeNull()
    expect(img().getAttribute('alt')).toBe('excited yes')
    expect(img().getAttribute('src')).toBe(ANIM)
  })

  it('words with the link keep their text, the GIF under it', () => {
    show(msg(`this is me ${LINK}`))
    expect(content()!.textContent).toContain('this is me')
    expect(gifs()).toHaveLength(1)
  })

  it('a GIPHY preview card never shows; other cards do', () => {
    const card = (url: string): LinkEmbed => ({ url, provider: 'generic', kind: 'card', provider_id: null, title: 'T', description: null, site_name: 'S', author: null, thumb: null, start: null, vertical: false })
    show(msg(`${LINK} https://example.com/a`, { embeds: [card(LINK), card('https://example.com/a')] }))
    const cards = [...document.querySelectorAll('.embed')]
    expect(cards).toHaveLength(1)
    expect(cards[0].innerHTML).not.toContain('giphy')
    expect(gifs()).toHaveLength(1)
  })

  it('under reduced motion: the still, playing on hover, and a click keeps it playing', () => {
    motion.reducedMotion = true
    show(picked())
    expect(img().getAttribute('src')).toBe(STILL)
    const play = gifs()[0].querySelector<HTMLButtonElement>('button.play')!
    play.dispatchEvent(new Event('pointerenter'))
    flushSync()
    expect(img().getAttribute('src')).toBe(ANIM)
    play.dispatchEvent(new Event('pointerleave'))
    flushSync()
    expect(img().getAttribute('src')).toBe(STILL)
    play.click()
    flushSync()
    expect(img().getAttribute('src')).toBe(ANIM)
    expect(play.getAttribute('aria-pressed')).toBe('true')
  })

  it('a click pauses a playing GIF', () => {
    show(picked())
    gifs()[0].querySelector<HTMLButtonElement>('button.play')!.click()
    flushSync()
    expect(img().getAttribute('src')).toBe(STILL)
  })

  it('the star adds it to your favourites', async () => {
    show(picked())
    await flush()
    gifs()[0].querySelector<HTMLButtonElement>('.star')!.click()
    await flush()
    expect(invoke).toHaveBeenCalledWith('set_gif_favourite', { gif: { id: ID, title: 'Excited Yes GIF', width: 480, height: 270 }, on: true })
    expect(gifFavs.list.map((g) => g.id)).toEqual([ID])
    expect(gifs()[0].querySelector('.star')!.getAttribute('aria-label')).toBe('Remove from favourite GIFs')
  })

  it('a GIF GIPHY can’t give falls back to its link', () => {
    show(msg(LINK))
    img().dispatchEvent(new Event('error'))
    flushSync()
    expect(gifs()).toHaveLength(0)
    expect(content()!.textContent).toContain(LINK)
  })

  it('with link previews off, a pasted link stays a link; a picked GIF still shows', () => {
    previews.show = false
    try {
      show(msg(LINK))
      expect(gifs()).toHaveLength(0)
      expect(content()!.textContent).toContain(LINK)
      unmount(cmp!)
      show(picked())
      expect(gifs()).toHaveLength(1)
    } finally {
      previews.show = true
    }
  })

  it('a deleted GIF message shows nothing of it', () => {
    show(picked({ deleted: true, content: '' }))
    expect(gifs()).toHaveLength(0)
  })
})
