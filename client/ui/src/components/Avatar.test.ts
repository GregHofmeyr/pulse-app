// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fakeMatchMedia, FakeIntersectionObserver } from '../lib/testMotion'

vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)

import Avatar from './Avatar.svelte'
import { app } from '../lib/store.svelte'
import { applyEvent, emptyState } from '../lib/state'
import { profileUi, closeCard } from '../lib/profileCard.svelte'
import { motion } from '../lib/animate.svelte'

const A = 'a'.repeat(64)
const S = 'b'.repeat(64)
const url = (h: string) => `pulse-media://localhost/${h}`
let cmp: ReturnType<typeof mount> | undefined
const origAdd = EventTarget.prototype.addEventListener
const origRemove = EventTarget.prototype.removeEventListener

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
  it('a touch tap does not leave a list avatar animating', () => {
    mountAv({}, { avatar_hash: A, avatar_still: S })
    const img = document.querySelector('img')!
    FakeIntersectionObserver.set(img, true)
    const av = img.closest('.av')!
    av.dispatchEvent(new Event('pointerenter'))
    flushSync()
    expect(img.getAttribute('src')).toBe(url(A))
    av.dispatchEvent(Object.assign(new Event('pointerup', { bubbles: true }), { pointerType: 'touch' }))
    flushSync()
    expect(img.getAttribute('src')).toBe(url(S))
    av.dispatchEvent(new Event('pointerenter'))
    av.dispatchEvent(new Event('pointercancel'))
    flushSync()
    expect(img.getAttribute('src')).toBe(url(S))
    // a mouse button release over it keeps it playing
    av.dispatchEvent(new Event('pointerenter'))
    av.dispatchEvent(Object.assign(new Event('pointerup', { bubbles: true }), { pointerType: 'mouse' }))
    flushSync()
    expect(img.getAttribute('src')).toBe(url(A))
  })
  it('only an animated avatar watches the pointer', () => {
    // the pointer listeners the avatar puts on its own element, live (added and not yet removed)
    const live = new Map<string, number>()
    const add = vi.spyOn(EventTarget.prototype, 'addEventListener').mockImplementation(function (this: EventTarget, type: string, ...rest: unknown[]) {
      if (this instanceof HTMLElement && this.classList.contains('av') && type.startsWith('pointer')) live.set(type, (live.get(type) ?? 0) + 1)
      return origAdd.call(this, type, ...(rest as [EventListener]))
    })
    const remove = vi.spyOn(EventTarget.prototype, 'removeEventListener').mockImplementation(function (this: EventTarget, type: string, ...rest: unknown[]) {
      if (this instanceof HTMLElement && this.classList.contains('av') && type.startsWith('pointer')) live.set(type, (live.get(type) ?? 0) - 1)
      return origRemove.call(this, type, ...(rest as [EventListener]))
    })
    const watching = () => [...live].filter(([, n]) => n > 0).map(([t]) => t).sort()
    try {
      for (const user of [{ avatar_hash: null, avatar_still: null }, { avatar_hash: A, avatar_still: null }, { avatar_hash: A, avatar_still: A }]) {
        mountAv({}, user)
        expect(watching(), JSON.stringify(user)).toEqual([])
        unmount(cmp!)
        cmp = undefined
        document.body.innerHTML = ''
      }
      mountAv({}, { avatar_hash: A, avatar_still: S })
      expect(watching()).toEqual(['pointercancel', 'pointerenter', 'pointerleave', 'pointerup'])
      // it stops once the picture no longer animates
      app.state = applyEvent(app.state, { t: 'ProfileUpdated', d: { user: { id: 'U2', username: 'sam', avatar_hash: A, avatar_still: null, style: { font: null, colors: [] } }, card: { banner_hash: null, banner_still: null, theme_top: '#3a3d46', theme_bottom: '#16171b', pronouns: null, bio: null } } }, 0)
      flushSync()
      expect(watching()).toEqual([])
    } finally {
      add.mockRestore()
      remove.mockRestore()
    }
  })
  it('swaps the picture live on a ProfileUpdated', () => {
    const N = 'c'.repeat(64)
    mountAv({ place: 'card' }, { avatar_hash: A, avatar_still: null })
    expect(document.querySelector('img')!.getAttribute('src')).toBe(url(A))
    app.state = applyEvent(app.state, { t: 'ProfileUpdated', d: { user: { id: 'U2', username: 'sam', avatar_hash: N, avatar_still: null, style: { font: null, colors: [] } }, card: { banner_hash: null, banner_still: null, theme_top: '#3a3d46', theme_bottom: '#16171b', pronouns: null, bio: null } } }, 0)
    flushSync()
    expect(document.querySelector('img')!.getAttribute('src')).toBe(url(N))
  })
  it('holds the still under reduced motion, even on a card', () => {
    motion.reducedMotion = true
    try {
      mountAv({ place: 'card' }, { avatar_hash: A, avatar_still: S })
      const img = document.querySelector('img')!
      FakeIntersectionObserver.set(img, true)
      img.closest('.av')!.dispatchEvent(new Event('pointerenter'))
      flushSync()
      expect(img.getAttribute('src')).toBe(url(S))
    } finally {
      motion.reducedMotion = false
    }
  })
  it('interactive avatar opens the card', () => {
    mountAv({ interactive: true, serverId: 'S1' }, { avatar_hash: null, avatar_still: null })
    const b = document.querySelector('button')!
    b.click()
    flushSync()
    expect(profileUi.card).toMatchObject({ userId: 'U2', serverId: 'S1' })
  })
})
