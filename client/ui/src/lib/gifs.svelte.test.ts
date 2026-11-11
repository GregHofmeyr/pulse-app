// Favourite GIFs (#58): a fetch that began before a star doesn't undo it on screen, one GIF's changes reach the
// server in order, and a refusal puts back only that GIF.
import { beforeEach, describe, expect, it, vi } from 'vitest'

type Call = { cmd: string; args: Record<string, unknown> | undefined; resolve: (v: unknown) => void; reject: (e: unknown) => void }
const calls: Call[] = []
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, args?: Record<string, unknown>) => new Promise((resolve, reject) => calls.push({ cmd, args, resolve, reject })),
}))

import { gifFavs, loadGifFavs, resetGifFavs, toggleGifFav } from './gifs.svelte'

const g = (id: string) => ({ id, title: id, width: 10, height: 10 })
const flush = async () => { for (let i = 0; i < 10; i++) await Promise.resolve() }
const pending = (cmd: string) => calls.filter((c) => c.cmd === cmd)

beforeEach(() => {
  resetGifFavs()
  calls.length = 0
})

describe('favourite GIFs', () => {
  it('a fetch begun before a star is dropped and asked again after it', async () => {
    const load = loadGifFavs('U1', true)
    const star = toggleGifFav(g('a'))
    expect(gifFavs.list.map((x) => x.id)).toEqual(['a'])
    pending('gif_favourites')[0].resolve([]) // read before the star
    await flush()
    expect(gifFavs.list.map((x) => x.id)).toEqual(['a'])
    pending('set_gif_favourite')[0].resolve(undefined)
    await star
    await flush()
    const again = pending('gif_favourites')
    expect(again).toHaveLength(2)
    again[1].resolve([g('a'), g('b')])
    await load
    await flush()
    expect(gifFavs.list.map((x) => x.id)).toEqual(['a', 'b'])
    expect(gifFavs.loaded).toBe(true)
  })

  it('a quick star then unstar goes out in that order', async () => {
    const one = toggleGifFav(g('a'))
    const two = toggleGifFav(g('a'))
    await flush()
    expect(pending('set_gif_favourite').map((c) => c.args!.on)).toEqual([true]) // the unstar waits
    pending('set_gif_favourite')[0].resolve(undefined)
    await one
    await flush()
    expect(pending('set_gif_favourite').map((c) => c.args!.on)).toEqual([true, false])
    pending('set_gif_favourite')[1].resolve(undefined)
    await two
    expect(gifFavs.list).toEqual([])
  })

  it('a refusal puts back only that GIF', async () => {
    const a = toggleGifFav(g('a'))
    const b = toggleGifFav(g('b'))
    await flush()
    const [ca, cb] = pending('set_gif_favourite')
    cb.resolve(undefined)
    await b
    ca.reject('no')
    await expect(a).rejects.toBe('no')
    expect(gifFavs.list.map((x) => x.id)).toEqual(['b'])
  })
})
