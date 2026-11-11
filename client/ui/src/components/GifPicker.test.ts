// @vitest-environment jsdom
// The GIF picker (#58): trending on opening, a search a moment after typing stops (and only the newest answer shown),
// more on scrolling, GIPHY's order and URLs as they came, the Favourites rail, the star, "not set up", GIPHY busy,
// "Powered by GIPHY", and a pick handing over the GIF and its link.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { GifPage, GifResult } from '../lib/tauri'
import type { Gif } from '../lib/protocol/Gif'

const r = (id: string, title = `${id} GIF`): GifResult => ({
  id, title, url: `https://giphy.com/gifs/fun-${id}`, width: 480, height: 270,
  preview: `https://media1.giphy.com/media/v1.x/${id}/200w.webp?cid=c&rid=200w.webp&ct=g`, preview_width: 200, preview_height: 113,
  still: `https://media1.giphy.com/media/v1.x/${id}/200w_s.gif?cid=c&rid=200w_s.gif&ct=g`,
})
let feed: (query: string, offset: number) => Promise<GifPage | null>
let favs: Gif[] = []
const invoke = vi.fn(async (cmd: string, args?: Record<string, unknown>): Promise<unknown> => {
  if (cmd === 'gif_feed') return feed(args!.query as string, args!.offset as number)
  if (cmd === 'gif_favourites') return favs
  return undefined
})
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import GifPicker, { SEARCH_DELAY_MS } from './GifPicker.svelte'
import { app, resetState } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import { gifFavs, resetGifFavs } from '../lib/gifs.svelte'

let cmp: ReturnType<typeof mount> | undefined
const picked: [Gif, string][] = []
let closed = 0
function open() {
  cmp = mount(GifPicker, { target: document.body, props: { onPick: (g: Gif, link: string) => picked.push([g, link]), onClose: () => closed++ } })
  flushSync()
}
const flush = async () => { for (let i = 0; i < 6; i++) await Promise.resolve(); flushSync() }
const picker = () => document.querySelector<HTMLElement>('[role="dialog"][aria-label="GIF picker"]')!
const search = () => picker().querySelector<HTMLInputElement>('input[aria-label="Search GIPHY"]')!
const cells = () => [...picker().querySelectorAll<HTMLButtonElement>('.pick')]
const labels = () => cells().map((c) => c.getAttribute('aria-label'))
const rail = (label: string) => picker().querySelector<HTMLButtonElement>(`.rail-btn[aria-label="${label}"]`)!
const text = () => picker().textContent!.replace(/\s+/g, ' ')
function type(q: string) {
  search().value = q
  search().dispatchEvent(new Event('input'))
  flushSync()
}
const feedCalls = () => invoke.mock.calls.filter(([c]) => c === 'gif_feed').map(([, a]) => a)

beforeEach(() => {
  vi.useFakeTimers()
  resetState()
  resetGifFavs()
  app.state = { ...emptyState(), me: { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } } }
  feed = async (q, offset) => ({ gifs: [r(`${q || 'T'}${offset}a`), r(`${q || 'T'}${offset}b`)], next: offset + 2 })
  favs = []
  invoke.mockClear()
  picked.length = 0
  closed = 0
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  vi.useRealTimers()
})

describe('GifPicker', () => {
  it('opens on trending, in GIPHY’s order, with GIPHY’s URLs untouched', async () => {
    open()
    await flush()
    expect(feedCalls()).toEqual([{ query: '', offset: 0 }])
    expect(labels().sort()).toEqual(['T0a GIF', 'T0b GIF'])
    const img = cells()[0].querySelector('img')!
    // on screen in a focused window it plays: GIPHY's preview, exactly as given
    expect(img.getAttribute('src')).toBe(r('T0a').preview)
    expect(document.activeElement).toBe(search())
  })

  it('says Powered by GIPHY', async () => {
    open()
    await flush()
    expect(picker().querySelector('.foot')!.textContent).toContain('Powered by GIPHY')
  })

  it('searches a moment after typing stops, and only the newest answer shows', async () => {
    open()
    await flush()
    type('c')
    type('ca')
    type('cats')
    await flush()
    expect(feedCalls()).toHaveLength(1) // still waiting
    vi.advanceTimersByTime(SEARCH_DELAY_MS)
    await flush()
    expect(feedCalls()).toEqual([{ query: '', offset: 0 }, { query: 'cats', offset: 0 }])
    expect(labels().sort()).toEqual(['cats0a GIF', 'cats0b GIF'])

    // a slow older answer landing after a newer one is dropped
    let release: (p: GifPage) => void = () => {}
    feed = (q) => (q === 'dogs' ? new Promise((res) => (release = res)) : Promise.resolve({ gifs: [r('birds')], next: null }))
    type('dogs')
    vi.advanceTimersByTime(SEARCH_DELAY_MS)
    type('birds')
    vi.advanceTimersByTime(SEARCH_DELAY_MS)
    await flush()
    release({ gifs: [r('dogs')], next: null })
    await flush()
    expect(labels()).toEqual(['birds GIF'])
  })

  it('loads the next page near the end of the grid, without showing a GIF twice', async () => {
    feed = async (_q, offset) => ({ gifs: offset ? [r('B'), r('C')] : [r('A'), r('B')], next: offset ? null : 2 })
    open()
    await flush()
    const grid = picker().querySelector<HTMLElement>('.grid')!
    Object.defineProperties(grid, { scrollHeight: { value: 500 }, clientHeight: { value: 420 }, scrollTop: { value: 0, writable: true } })
    grid.dispatchEvent(new Event('scroll'))
    await flush()
    expect(feedCalls()).toEqual([{ query: '', offset: 0 }, { query: '', offset: 2 }])
    expect(labels().sort()).toEqual(['A GIF', 'B GIF', 'C GIF'])
    // the end: no more asking
    grid.dispatchEvent(new Event('scroll'))
    await flush()
    expect(feedCalls()).toHaveLength(2)
  })

  it('a pick hands over the GIF and its GIPHY page', async () => {
    open()
    await flush()
    const first = cells().find((c) => c.getAttribute('aria-label') === 'T0a GIF')!
    first.click()
    expect(picked).toEqual([[{ id: 'T0a', title: 'T0a GIF', width: 480, height: 270 }, 'https://giphy.com/gifs/fun-T0a']])
  })

  it('Enter sends the first result of what’s typed, never the old results while the search is on its way', async () => {
    open()
    await flush()
    type('cats')
    search().dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    expect(picked).toHaveLength(0) // trending still showing
    vi.advanceTimersByTime(SEARCH_DELAY_MS)
    await flush()
    search().dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    expect(picked.map(([g]) => g.id)).toEqual(['cats0a'])
  })

  it('a failed page stops loading more until Try again', async () => {
    feed = async (_q, offset) => {
      if (offset) throw 'GIF search is busy right now. Try again in a minute.'
      return { gifs: [r('A'), r('B')], next: 2 }
    }
    open()
    await flush()
    const grid = picker().querySelector<HTMLElement>('.grid')!
    Object.defineProperties(grid, { scrollHeight: { value: 500 }, clientHeight: { value: 420 }, scrollTop: { value: 0, writable: true } })
    grid.dispatchEvent(new Event('scroll'))
    await flush()
    grid.dispatchEvent(new Event('scroll'))
    grid.dispatchEvent(new Event('scroll'))
    await flush()
    expect(feedCalls()).toHaveLength(2)
    expect(text()).toContain('GIF search is busy')
    feed = async () => ({ gifs: [r('C')], next: null })
    ;[...picker().querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent === 'Try again')!.click()
    await flush()
    expect(feedCalls()).toEqual([{ query: '', offset: 0 }, { query: '', offset: 2 }, { query: '', offset: 2 }])
    expect(labels().sort()).toEqual(['A GIF', 'B GIF', 'C GIF'])
  })

  it('Escape closes it', async () => {
    open()
    await flush()
    search().dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    expect(closed).toBe(1)
  })

  it('without a key on the server: GIFs aren’t set up', async () => {
    feed = async () => null
    open()
    await flush()
    expect(text()).toContain("GIFs aren't set up on this server yet")
    expect(search().disabled).toBe(true)
    expect(cells()).toHaveLength(0)
  })

  it('GIPHY busy says so', async () => {
    feed = async () => { throw 'GIF search is busy right now. Try again in a minute.' }
    open()
    await flush()
    expect(text()).toContain('GIF search is busy right now. Try again in a minute.')
  })

  it('no results says so', async () => {
    open()
    await flush()
    feed = async () => ({ gifs: [], next: null })
    type('zzzz')
    vi.advanceTimersByTime(SEARCH_DELAY_MS)
    await flush()
    expect(text()).toContain('No GIFs found')
  })

  it('Favourites shows yours, drawn from GIPHY’s media links for each id', async () => {
    favs = [{ id: 'fav1', title: 'Fav one', width: 300, height: 200 }]
    open()
    await flush()
    expect(invoke.mock.calls.some(([c]) => c === 'gif_favourites')).toBe(true)
    rail('Favourites').click()
    flushSync()
    expect(labels()).toEqual(['Fav one'])
    expect(cells()[0].querySelector('img')!.getAttribute('src')).toBe('https://media.giphy.com/media/fav1/200w.webp')
    cells()[0].click()
    expect(picked[0]).toEqual([favs[0], 'https://giphy.com/gifs/fav1'])
    // typing a search goes back to GIPHY's results
    type('cats')
    expect(rail('Search results').getAttribute('aria-pressed')).toBe('true')
  })

  it('no favourites yet says how to add one', async () => {
    open()
    await flush()
    rail('Favourites').click()
    flushSync()
    expect(text()).toContain('Star a GIF to keep it here')
  })

  it('the star adds a GIF to your favourites and takes it off again', async () => {
    open()
    await flush()
    const star = picker().querySelector<HTMLButtonElement>('.cell .star[aria-label="Add to favourite GIFs"]')!
    star.click()
    await flush()
    expect(invoke).toHaveBeenCalledWith('set_gif_favourite', { gif: expect.objectContaining({ id: expect.stringMatching(/^T0/) }), on: true })
    expect(gifFavs.list).toHaveLength(1)
    picker().querySelector<HTMLButtonElement>('.cell .star[aria-label="Remove from favourite GIFs"]')!.click()
    await flush()
    expect(invoke).toHaveBeenLastCalledWith('set_gif_favourite', expect.objectContaining({ on: false }))
    expect(gifFavs.list).toHaveLength(0)
  })

  it('a refused star is put back and says why', async () => {
    open()
    await flush()
    invoke.mockImplementationOnce(async () => { throw 'You can keep 200 favourite GIFs. Unstar one to make room.' })
    picker().querySelector<HTMLButtonElement>('.cell .star')!.click()
    await flush()
    await flush()
    expect(gifFavs.list).toHaveLength(0)
    expect(text()).toContain('You can keep 200 favourite GIFs')
  })
})
