// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fakeMatchMedia, FakeIntersectionObserver } from '../../lib/testMotion'

vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))

import RingPopover from './RingPopover.svelte'
import { ProfileEditorState } from '../../lib/profileEditor.svelte'
import { RING_COLORS, RING_STYLES } from '../../lib/speakingRing'
import { ringPrefs } from '../../lib/speakingRing.svelte'
import { motion } from '../../lib/animate.svelte'
import { app } from '../../lib/store.svelte'
import { emptyState } from '../../lib/state'
import type { Profile } from '../../lib/protocol/Profile'
import type { User } from '../../lib/protocol/User'

const alex: User = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const profile: Profile = {
  user: alex,
  card: { banner_hash: null, banner_still: null, theme_top: '#3a3d46', theme_bottom: '#16171b', pronouns: null, bio: null },
  stats: { voice_seconds: 0, member_since: '2026-10-03T00:00:00Z', top_reaction: null, times_renamed: 0 },
  badges: [], name_archive: [], showcase: [], showcase_chosen: false,
}
let cmp: ReturnType<typeof mount> | undefined
let st: ProfileEditorState
const onClose = vi.fn()
beforeEach(() => {
  vi.useFakeTimers()
  onClose.mockReset()
  app.state = { ...emptyState(), me: alex, people: { U1: { user: alex, online: true, last_seen_at: null } } } as never
  st = new ProfileEditorState(profile)
  cmp = mount(RingPopover, { target: document.body, props: { state: st, onClose } })
  flushSync()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  motion.reducedMotion = false
  ringPrefs.classic = false
  vi.useRealTimers()
})
const click = (el: Element) => { (el as HTMLElement).click(); flushSync() }
const q = <T extends Element = HTMLElement>(sel: string) => document.querySelector<T>(sel)!
const stage = () => q('.ring-stage')
const seen = () => { FakeIntersectionObserver.set(stage(), true); flushSync() }
const tick = (ms: number) => { vi.advanceTimersByTime(ms); flushSync() }

describe('the Speaking ring popover', () => {
  it('two_groups_still_or_looping_and_follows_your_voice', () => {
    const heads = [...document.querySelectorAll('.tiles h3')].map((h) => h.textContent?.trim())
    expect(heads).toEqual(['Still or looping', 'Follows your voice', 'Colour'])
    const group = (name: string) => [...q(`[role=group][aria-label="${name}"]`).querySelectorAll('[role=radio]')].map((r) => r.getAttribute('aria-label'))
    expect(group('Still or looping')).toEqual(RING_STYLES.filter((r) => !r.reactive).map((r) => r.label))
    expect(group('Follows your voice')).toEqual(RING_STYLES.filter((r) => r.reactive).map((r) => r.label))
    // each style tile draws its own ring, still, whatever the viewer's Classic-for-all says
    ringPrefs.classic = true
    flushSync()
    const orbit = q('[role=radio][aria-label="Orbit"] .ring-box')
    expect(orbit.dataset.style).toBe('orbit')
    expect(orbit.dataset.ring).toBe('still')
    click(q('[role=radio][aria-label="Orbit"]'))
    expect(st.draft.ringStyle).toBe('orbit')
    expect(q('[role=radio][aria-label="Orbit"]').getAttribute('aria-checked')).toBe('true')
  })
  it('colour_row_includes_profile_colour', () => {
    const row = q('[role=radiogroup][aria-label="Ring colour"]')
    expect([...row.querySelectorAll('[role=radio]')].map((r) => r.getAttribute('aria-label'))).toEqual(RING_COLORS.map((c) => c.label))
    expect(row.querySelector('[aria-label="Profile colour"]')).not.toBeNull()
    click(row.querySelector('[aria-label="Gold"]')!)
    expect(st.draft.ringColor).toBe('gold')
    click(row.querySelector('[aria-label="Profile colour"]')!)
    expect(st.draft.ringColor).toBe('profile')
  })
  it('preview_runs_envelope_only_when_reactive_and_motion_allowed', () => {
    st.draft.ringStyle = 'orbit'
    flushSync()
    seen()
    tick(300)
    expect(stage().style.getPropertyValue('--lvl')).toBe('') // not reactive
    st.draft.ringStyle = 'liquid'
    flushSync()
    tick(300)
    expect(stage().style.getPropertyValue('--lvl')).not.toBe('') // reactive and seen
    st.draft.ringStyle = 'orbit'
    flushSync()
    expect(stage().style.getPropertyValue('--lvl')).toBe('') // stopped and cleared
    st.draft.ringStyle = 'bars'
    motion.reducedMotion = true
    flushSync()
    tick(300)
    expect(stage().style.getPropertyValue('--lvl')).toBe('') // reduced motion: no envelope
    expect(q('.ring-stage .ring-box').dataset.ring).toBe('still')
    motion.reducedMotion = false
    flushSync()
    tick(300)
    expect(stage().style.getPropertyValue('--lvl')).not.toBe('')
    FakeIntersectionObserver.set(stage(), false)
    flushSync()
    expect(stage().style.getPropertyValue('--lvl')).toBe('') // off screen: not running
  })
  it('cancel_restores_style_and_colour', () => {
    st.draft.ringStyle = 'sweep'
    st.draft.ringColor = 'cyan'
    flushSync()
    unmount(cmp!)
    cmp = mount(RingPopover, { target: document.body, props: { state: st, onClose } })
    flushSync()
    click(q('[role=radio][aria-label="Bars round the ring"]'))
    click(q('[role=radio][aria-label="Pink"]'))
    expect([st.draft.ringStyle, st.draft.ringColor]).toEqual(['bars', 'pink'])
    click(q('button[aria-label="Cancel"]'))
    expect([st.draft.ringStyle, st.draft.ringColor]).toEqual(['sweep', 'cyan'])
    expect(onClose).toHaveBeenCalledTimes(1)
  })
  it('apply_keeps_the_choice_and_only_closes', () => {
    click(q('[role=radio][aria-label="Halo"]'))
    click(q('button[aria-label="Apply"]'))
    expect(st.draft.ringStyle).toBe('halo')
    expect(onClose).toHaveBeenCalledTimes(1)
  })
})
