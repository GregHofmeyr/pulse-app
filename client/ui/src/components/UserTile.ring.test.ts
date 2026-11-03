// @vitest-environment jsdom
// "Who played it": a soundboard sound lights the player's tile ring like speaking does, without being speaking.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: async () => undefined }))

import UserTile from './UserTile.svelte'
import { voice } from '../lib/voice.svelte'
import { callChanged, lightRing } from '../lib/soundboard.svelte'

let cmp: ReturnType<typeof mount> | undefined
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  callChanged(null)
})

it("a playing sound lights the player's ring and leaves speaking alone", () => {
  vi.useFakeTimers()
  try {
    const entry = { userId: 'U2', name: 'sam', muted: false, deafened: false, pending: false }
    cmp = mount(UserTile, { target: document.body, props: { entry, onMenu: () => {} } })
    flushSync()
    const tile = document.querySelector('.tile')!
    expect(tile.classList.contains('speaking')).toBe(false)
    lightRing('U2', 800)
    flushSync()
    expect(tile.classList.contains('speaking')).toBe(true)
    expect(voice.speaking.has('U2')).toBe(false)
    vi.advanceTimersByTime(900)
    flushSync()
    expect(tile.classList.contains('speaking')).toBe(false)
  } finally {
    vi.useRealTimers()
  }
})
