// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import AppearanceSettings from './AppearanceSettings.svelte'
import Name from './Name.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import { appearance, setShowNameStyles } from '../lib/appearance.svelte'

const cmps: ReturnType<typeof mount>[] = []
beforeEach(() => {
  localStorage.clear()
  setShowNameStyles(true)
  app.state = { ...emptyState(), members: { S1: [{ user: { id: 'U2', username: 'sam', avatar_hash: null, avatar_still: null, style: { font: 'fredoka', colors: ['#ffc4a3'] } }, nickname: null }] } } as never
})
afterEach(() => {
  cmps.splice(0).forEach((c) => unmount(c))
  document.body.innerHTML = ''
  localStorage.clear()
})

describe('AppearanceSettings', () => {
  it('the switch turns name styles off everywhere', () => {
    cmps.push(mount(Name, { target: document.body, props: { userId: 'U2', serverId: 'S1', interactive: false } }))
    cmps.push(mount(AppearanceSettings, { target: document.body }))
    flushSync()
    const name = document.querySelector<HTMLElement>('.name')!
    expect(name.getAttribute('style')).toContain('Fredoka')
    const sw = document.querySelector<HTMLInputElement>('input[type=checkbox][role=switch]')!
    expect(sw.checked).toBe(true)
    expect(document.body.textContent).toContain('Show name styles')
    expect(document.body.textContent).toContain("Show everyone's chosen name fonts and colours. Off shows plain names (role colours still show).")
    sw.click()
    flushSync()
    expect(appearance.showNameStyles).toBe(false)
    expect(localStorage.getItem('pulse.showNameStyles')).toBe('false')
    expect(name.getAttribute('style')).not.toContain('Fredoka')
    expect(name.getAttribute('style')).not.toContain('color')
  })
})
