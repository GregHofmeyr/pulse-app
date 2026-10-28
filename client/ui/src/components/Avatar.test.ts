// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fakeMatchMedia, FakeIntersectionObserver } from '../lib/testMotion'

vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)

import Avatar from './Avatar.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import { profileUi, closeCard } from '../lib/profileCard.svelte'
import { motion } from '../lib/animate.svelte'

const A = 'a'.repeat(64)
const S = 'b'.repeat(64)
const url = (h: string) => `pulse-media://localhost/${h}`
let cmp: ReturnType<typeof mount> | undefined

beforeEach(() => {
  app.state = emptyState()
  motion.streaming = false
  window.dispatchEvent(new Event('focus'))
  closeCard()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})
function mountAv(props: Record<string, unknown>, user: { avatar_hash: string | null; avatar_still: string | null } | null) {
  if (user) app.state.people = { U2: { user: { id: 'U2', username: 'sam', style: { font: null, colors: [] }, ...user } } } as never
  cmp = mount(Avatar, { target: document.body, props: { id: 'U2', name: 'sam', ...props } })
  flushSync()
}

describe('Avatar', () => {
  it('initial when no avatar', () => {
    mountAv({}, { avatar_hash: null, avatar_still: null })
    expect(document.querySelector('img')).toBeNull()
    expect(document.querySelector('.av')!.textContent!.trim()).toBe('S')
  })
  it('initial for unknown users, with the presence dot', () => {
    mountAv({ online: true }, null)
    expect(document.querySelector('img')).toBeNull()
    expect(document.querySelector('.pres')).not.toBeNull()
  })
  it('still src in a list, animated src on a card', () => {
    mountAv({}, { avatar_hash: A, avatar_still: S })
    const img = document.querySelector('img')!
    FakeIntersectionObserver.set(img, true)
    flushSync()
    expect(img.getAttribute('src')).toBe(url(S))
    img.closest('.av')!.dispatchEvent(new Event('pointerenter'))
    flushSync()
    expect(img.getAttribute('src')).toBe(url(A))
    unmount(cmp!)
    document.body.innerHTML = ''
    mountAv({ place: 'card' }, { avatar_hash: A, avatar_still: S })
    const c = document.querySelector('img')!
    FakeIntersectionObserver.set(c, true)
    flushSync()
    expect(c.getAttribute('src')).toBe(url(A))
  })
  it('a still-only avatar uses its hash', () => {
    mountAv({ place: 'card' }, { avatar_hash: A, avatar_still: null })
    const img = document.querySelector('img')!
    FakeIntersectionObserver.set(img, true)
    flushSync()
    expect(img.getAttribute('src')).toBe(url(A))
  })
  it('interactive avatar opens the card', () => {
    mountAv({ interactive: true, serverId: 'S1' }, { avatar_hash: null, avatar_still: null })
    const b = document.querySelector('button')!
    b.click()
    flushSync()
    expect(profileUi.card).toMatchObject({ userId: 'U2', serverId: 'S1' })
  })
})
