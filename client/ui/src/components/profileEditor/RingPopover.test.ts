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
import { windowFocus } from '../../lib/windowFocus.svelte'
import { speakCss } from '../../lib/speakingRing'
import { previewUser, draftFrom } from '../../lib/profileDraft'
import { nameBg } from '../../lib/theme.svelte'
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
  motion.streaming = false
  windowFocus.focused = true
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

// Ported from ProfileSettings.test.ts (the old Speaking ring tab) when the old page went (#727, task 10).
describe('the Speaking ring popover, as the old tab behaved', () => {
  const caption = () => q('.caption').textContent!.trim()
  const radios = (label: string) => [...q(`[role=radiogroup][aria-label="${label}"]`).querySelectorAll<HTMLButtonElement>('[role=radio]')]
  const remount = (p: Profile) => {
    unmount(cmp!)
    document.body.innerHTML = ''
    st = new ProfileEditorState(p)
    cmp = mount(RingPopover, { target: document.body, props: { state: st, onClose } })
    flushSync()
  }

  it('the_help_is_a_tooltip_on_the_caption_not_text', () => {
    const tip = q('.caption').dataset.tip!
    expect(tip).toContain('How your avatar lights up when you talk. Everyone in the call sees it.')
    expect(tip).toContain('With reduced motion on, everyone sees the still version of each ring. In big calls some rings show still to keep things smooth.')
    expect(document.body.textContent).not.toContain('How your avatar lights up')
  })

  it('the_caption_says_why_the_preview_is_still', () => {
    st.draft.ringStyle = 'bars'
    flushSync()
    seen()
    expect(caption()).toBe('Preview · talking')
    motion.reducedMotion = true
    flushSync()
    expect(caption()).toBe('Preview · still (reduced motion is on)')
    motion.reducedMotion = false
    motion.streaming = true
    flushSync()
    // drawn still while you are live, and no envelope runs
    expect(q('.ring-stage .ring-box').dataset.ring).toBe('still')
    expect(caption()).toBe("Preview · still (you're live)")
    tick(1000)
    expect(stage().style.getPropertyValue('--lvl')).toBe('')
    // going off air: it talks again
    motion.streaming = false
    flushSync()
    expect(caption()).toBe('Preview · talking')
    tick(300)
    expect(stage().style.getPropertyValue('--lvl')).not.toBe('')
  })

  it('the_envelope_stops_while_the_window_is_unfocused', () => {
    st.draft.ringStyle = 'dots'
    flushSync()
    seen()
    tick(300)
    expect(stage().style.getPropertyValue('--lvl')).not.toBe('')
    windowFocus.focused = false
    flushSync()
    expect(stage().style.getPropertyValue('--lvl')).toBe('')
    expect(vi.getTimerCount()).toBe(0)
    windowFocus.focused = true
    flushSync()
    tick(300)
    expect(stage().style.getPropertyValue('--lvl')).not.toBe('')
  })

  it('style_and_colour_radios_have_one_tab_stop_and_the_arrows_cross_groups_and_pick', () => {
    expect(radios('Ring style')).toHaveLength(RING_STYLES.length)
    expect(radios('Ring style').filter((r) => r.tabIndex === 0).map((r) => r.getAttribute('aria-label'))).toEqual(['Classic'])
    expect(radios('Ring style').find((r) => r.getAttribute('aria-label') === 'Classic')!.getAttribute('aria-checked')).toBe('true')
    // the last of "Still or looping", then the arrow lands on the first of "Follows your voice"
    const still = RING_STYLES.filter((r) => !r.reactive)
    const voice = RING_STYLES.filter((r) => r.reactive)
    click(q(`[role=radio][aria-label="${still[still.length - 1].label}"]`))
    const last = q<HTMLElement>(`[role=radio][aria-label="${still[still.length - 1].label}"]`)
    last.focus()
    last.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true, cancelable: true }))
    flushSync()
    const next = q<HTMLElement>(`[role=radio][aria-label="${voice[0].label}"]`)
    expect(document.activeElement).toBe(next)
    expect(next.getAttribute('aria-checked')).toBe('true')
    expect(st.draft.ringStyle).toBe(voice[0].key)
    expect(radios('Ring style').filter((r) => r.tabIndex === 0)).toEqual([next])
    // the preview draws the draft
    expect(q('.ring-stage .ring-box').dataset.style).toBe(voice[0].key)
    // colours: by name, one tab stop
    expect(radios('Ring colour').map((b) => b.getAttribute('aria-label'))).toEqual(RING_COLORS.map((c) => c.label))
    expect(radios('Ring colour').filter((r) => r.tabIndex === 0).map((r) => r.getAttribute('aria-label'))).toEqual(['Green'])
    expect(q('[aria-label="Pulse blue"]').style.getPropertyValue('--swatch')).toBe('var(--speak-blue)')
    click(q('[aria-label="Cyan"]'))
    expect(q('.ring-stage .ring-box').style.getPropertyValue('--speak')).toBe('var(--speak-cyan)')
  })

  it('the_profile_swatch_tip_says_where_the_colour_comes_from', () => {
    const coloured = { ...alex, style: { font: null, colors: ['#ffbe7a'] } }
    remount({ ...profile, user: coloured })
    const prof = q('[aria-label="Profile colour"]')
    expect(prof.dataset.tip).toBe('Your name colour, from Name style')
    const want = speakCss(previewUser(alex, draftFrom({ ...profile, user: coloured })), 'profile', nameBg('side'))
    expect(prof.style.getPropertyValue('--swatch')).toBe(want)
    remount(profile)
    expect(q('[aria-label="Profile colour"]').dataset.tip).toMatch(/ You haven't set one, so it's green\.$/)
    expect(q('[aria-label="Profile colour"]').style.getPropertyValue('--swatch')).toBe('var(--speak-green)')
  })

  it('a_saved_ring_is_what_the_popover_shows', () => {
    remount({ ...profile, user: { ...alex, ring_style: 'dots', ring_color: 'gold' } })
    expect(q('[aria-label="Wave of dots"]').getAttribute('aria-checked')).toBe('true')
    expect(q('[aria-label="Gold"]').getAttribute('aria-checked')).toBe('true')
    expect(q('.ring-stage .ring-box').dataset.style).toBe('dots')
  })
})
