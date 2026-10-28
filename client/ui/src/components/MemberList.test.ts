// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fakeMatchMedia, FakeIntersectionObserver } from '../lib/testMotion'

vi.mock('@tauri-apps/api/core', () => ({ invoke: async () => undefined }))
vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)

import MemberList from './MemberList.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import { appearance } from '../lib/appearance.svelte'
import { motion } from '../lib/animate.svelte'
import { closeCard, profileUi } from '../lib/profileCard.svelte'

const A = 'a'.repeat(64)
const S = 'b'.repeat(64)
const url = (h: string) => `pulse-media://localhost/${h}`
const alex = { id: 'U1', username: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const sam = { id: 'U2', username: 'sam', avatar_hash: A, avatar_still: S, style: { font: 'fredoka', colors: ['#ffc4a3'] } }

let cmp: ReturnType<typeof mount> | undefined
beforeEach(() => {
  appearance.showNameStyles = true
  motion.streaming = false
  window.dispatchEvent(new Event('focus'))
  closeCard()
  app.state = { ...emptyState(), me: alex, members: { S1: [{ user: alex, nickname: null }, { user: sam, nickname: 'Sammy' }] } } as never
  cmp = mount(MemberList, { target: document.body, props: { serverId: 'S1', onMessage: () => {} } })
  flushSync()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  closeCard()
})
const row = (n: number) => document.querySelectorAll<HTMLElement>('.members .row')[n]

describe('MemberList', () => {
  it('rows use Name and Avatar', () => {
    const r = row(1)
    const name = r.querySelector<HTMLElement>('.name')!
    expect(name.textContent).toBe('Sammy')
    expect(name.getAttribute('style')).toContain('Fredoka')
    expect(name.getAttribute('style')).toContain('rgb(255, 196, 163)')
    const img = r.querySelector('img')!
    FakeIntersectionObserver.set(img, true)
    flushSync()
    expect(img.getAttribute('src')).toBe(url(S))
    name.click()
    flushSync()
    expect(profileUi.card).toMatchObject({ userId: 'U2', serverId: 'S1' })
  })
  it('the avatar animates while the pointer is over its row', () => {
    const r = row(1)
    const img = r.querySelector('img')!
    FakeIntersectionObserver.set(img, true)
    flushSync()
    r.dispatchEvent(new Event('pointerenter'))
    flushSync()
    expect(img.getAttribute('src')).toBe(url(A))
    r.dispatchEvent(new Event('pointerleave'))
    flushSync()
    expect(img.getAttribute('src')).toBe(url(S))
  })
  it('a profile change restyles the row live', () => {
    app.state = { ...app.state, members: { S1: [app.state.members.S1[0], { user: { ...sam, style: { font: null, colors: ['#a3d8ff'] } }, nickname: 'Sammy' }] } }
    flushSync()
    expect(row(1).querySelector('.name')!.getAttribute('style')).toContain('rgb(163, 216, 255)')
  })
})
