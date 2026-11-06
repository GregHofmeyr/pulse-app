// @vitest-environment jsdom
// On the stage (#222) the avatar, name and mute badge grow with the tile; elsewhere (the full view's strip) they don't.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: async () => undefined }))

import UserTile from './UserTile.svelte'
import { tileScale } from '../lib/stagelayout'

let cmp: ReturnType<typeof mount> | undefined
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})
const muted = { userId: 'U2', name: 'sam', muted: true, deafened: false, pending: false }

it('a tall tile has a big avatar, name and badge', () => {
  cmp = mount(UserTile, { target: document.body, props: { entry: muted, height: 400, onMenu: () => {} } })
  flushSync()
  const s = tileScale(400)
  expect(document.querySelector<HTMLElement>('.av')!.style.width).toBe(`${s.avatar}px`)
  expect(document.querySelector<HTMLElement>('.name')!.style.fontSize).toBe(`${s.font}px`)
  expect(document.querySelector<HTMLElement>('.tile')!.style.getPropertyValue('--badge')).toBe(`${s.badge}px`)
})

it('without a height it keeps its fixed size', () => {
  cmp = mount(UserTile, { target: document.body, props: { entry: muted, onMenu: () => {} } })
  flushSync()
  expect(document.querySelector<HTMLElement>('.av')!.style.width).toBe('84px')
  expect(document.querySelector<HTMLElement>('.name')!.style.fontSize).toBe('')
})
