// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fakeMatchMedia, FakeIntersectionObserver } from '../lib/testMotion'

vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)

import PeoplePicker from './PeoplePicker.svelte'
import { app } from '../lib/store.svelte'
import { applyEvent, emptyState } from '../lib/state'

const PIC = 'a'.repeat(64)
const NEW = 'c'.repeat(64)
const url = (h: string) => `pulse-media://localhost/${h}`
const user = (id: string, username: string, avatar_hash: string | null = null) => ({ id, username, avatar_hash, avatar_still: null, style: { font: null, colors: [] } })
const card = { banner_hash: null, banner_still: null, theme_top: '#3a3d46', theme_bottom: '#16171b', pronouns: null, bio: null }

let cmp: ReturnType<typeof mount> | undefined
beforeEach(() => {
  app.state = emptyState()
  app.state.me = user('U1', 'alex') as never
  app.state.people = {
    U2: { user: user('U2', 'sam', PIC), online: true },
    U3: { user: user('U3', 'kim'), online: false },
  } as never
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})
const mountPicker = (fixed: string[] = []) => {
  cmp = mount(PeoplePicker, { target: document.body, props: { title: 'Add people', fixed, action: 'Add', onDone: () => {}, onClose: () => {} } })
  flushSync()
}
const chip = (name: string) => [...document.querySelectorAll<HTMLElement>('.chips .chip')].find((c) => c.textContent?.includes(name))!

describe('PeoplePicker chips', () => {
  it("a fixed chip and a picked chip show the person's avatar, picture or initial", () => {
    mountPicker(['U2'])
    expect(chip('sam').querySelector('.av img')?.getAttribute('src')).toBe(url(PIC))
    ;[...document.querySelectorAll<HTMLButtonElement>('.pick')].find((b) => b.textContent?.includes('kim'))!.click()
    flushSync()
    const kim = chip('kim')
    expect(kim.querySelector('.av img')).toBeNull()
    expect(kim.querySelector('.av')!.textContent!.trim()).toBe('K')
    expect(kim.getAttribute('aria-label')).toBe('Remove kim')
  })

  it("a chip's picture updates live on a ProfileUpdated", () => {
    mountPicker(['U2'])
    app.state = applyEvent(app.state, { t: 'ProfileUpdated', d: { user: user('U2', 'sam', NEW), card } }, 0)
    flushSync()
    expect(chip('sam').querySelector('.av img')?.getAttribute('src')).toBe(url(NEW))
  })
})
