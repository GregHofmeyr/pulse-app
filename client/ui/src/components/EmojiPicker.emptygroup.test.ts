// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: async () => undefined }))
// A group with no emoji (e.g. all of it lacks a Twemoji graphic) must not break the rail.
vi.mock('../lib/emoji', async (orig) => {
  const m = await orig<typeof import('../lib/emoji')>()
  return { ...m, standard: m.standard.filter((e) => e.g !== 1) }
})

import EmojiPicker from './EmojiPicker.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import { groups } from '../lib/emoji'

let cmp: ReturnType<typeof mount> | undefined
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

it('empty_group_renders_a_rail_button_without_an_icon', () => {
  app.state = emptyState()
  Element.prototype.scrollIntoView = vi.fn()
  cmp = mount(EmojiPicker, { target: document.body, props: { onPick: vi.fn(), onClose: vi.fn(), servers: [], memberOf: new Set<string>() } })
  flushSync()
  const btn = document.querySelector(`[data-rail="group:${groups[1].key}"]`)
  expect(btn).not.toBeNull()
  expect(btn!.querySelector('img')).toBeNull()
  expect(document.querySelectorAll('[data-rail]').length).toBe(groups.length)
})
