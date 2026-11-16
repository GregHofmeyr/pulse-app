// @vitest-environment jsdom
// A voice tile's game backdrop (#380): someone playing a game with art has it dimmed behind their avatar; no game, or a
// game with no art, is the plain tile.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: async () => undefined }))

import UserTile from './UserTile.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import type { Playing } from '../lib/protocol/Playing'

const sam = { id: 'U2', username: 'sam', display_name: 'sam', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const entry = { userId: 'U2', name: 'sam', muted: false, deafened: false, pending: false }
let cmp: ReturnType<typeof mount> | undefined
const withGame = (playing?: Playing) => {
  app.state = { ...emptyState(), people: { U2: { user: sam, online: true, last_seen_at: null, status: 'online', playing } } }
}
const show = () => {
  cmp = mount(UserTile, { target: document.body, props: { entry, onMenu: () => {} } })
  flushSync()
}
beforeEach(() => withGame())
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

it('a game with art: dimmed behind the avatar, hidden from screen readers', () => {
  withGame({ name: 'Space Game', steam_app_id: 570, started_at: null, art: { hero: { hash: 'a'.repeat(64), width: 480, height: 155 }, header: null, logo: null } })
  show()
  const img = document.querySelector<HTMLImageElement>('.tile > img.backdrop')!
  expect(img.getAttribute('src')).toContain('a'.repeat(64))
  expect(img.getAttribute('aria-hidden')).toBe('true')
  expect(img.getAttribute('alt')).toBe('')
  // first in the tile: everything else paints over it
  expect(document.querySelector('.tile')!.firstElementChild).toBe(img)
})

it('and it follows the game: gone when they stop', () => {
  withGame({ name: 'Hat Game', steam_app_id: 440, started_at: null, art: { hero: null, header: { hash: 'b'.repeat(64), width: 460, height: 215 }, logo: null } })
  show()
  expect(document.querySelector('img.backdrop')!.getAttribute('src')).toContain('b'.repeat(64))
  withGame()
  flushSync()
  expect(document.querySelector('img.backdrop')).toBeNull()
})

it('a game without art (not Steam, or art off): the plain tile', () => {
  withGame({ name: 'Old Game', steam_app_id: null, started_at: null, art: null })
  show()
  expect(document.querySelector('img.backdrop')).toBeNull()
})
