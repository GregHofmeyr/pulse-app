// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fakeMatchMedia, FakeIntersectionObserver } from '../lib/testMotion'

vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))

import { invoke } from '@tauri-apps/api/core'
import ProfileSettings from './ProfileSettings.svelte'
import Settings from './Settings.svelte'
import { app, resetState } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import type { User } from '../lib/protocol/User'
import type { Profile } from '../lib/protocol/Profile'
import type { UpdateProfileRequest } from '../lib/protocol/UpdateProfileRequest'
import type { MediaInfo } from '../lib/protocol/MediaInfo'
import type { Badge } from '../lib/protocol/Badge'
import { motion } from '../lib/animate.svelte'
import { windowFocus } from '../lib/windowFocus.svelte'
import { RING_COLORS, speakCss } from '../lib/speakingRing'
import { previewUser, draftFrom } from '../lib/profileDraft'
import { nameBg } from '../lib/theme.svelte'

const alex: User = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const saved = (over: Partial<Profile> = {}): Profile => ({
  user: alex,
  card: { banner_hash: null, banner_still: null, theme_top: '#3a3d46', theme_bottom: '#16171b', pronouns: 'they/them', bio: 'hello' },
  stats: { voice_seconds: 0, member_since: '2026-10-03T00:00:00Z', top_reaction: null, times_renamed: 0 },
  badges: [],
  name_archive: [],
  showcase: [],
  showcase_chosen: false,
  ...over,
})
const media = (hash: string, still: string | null = null): MediaInfo => ({ hash, content_type: 'image/webp', width: 256, height: 256, frames: still ? 10 : 1, animated: !!still, still_hash: still })

type Handler = (args: unknown, opts?: unknown) => unknown
let handlers: Record<string, Handler>
const calls = (cmd: string) => vi.mocked(invoke).mock.calls.filter((c) => c[0] === cmd)

let cmp: ReturnType<typeof mount> | undefined
async function open(p: Profile = saved()) {
  handlers.get_profile = () => p
  app.state = {
    ...emptyState(),
    me: alex,
    people: { U1: { user: alex, online: true, last_seen_at: null } },
  } as never
  const state = { dirty: false }
  cmp = mount(ProfileSettings, { target: document.body, props: { get dirty() { return state.dirty }, set dirty(v: boolean) { state.dirty = v } } })
  await vi.waitFor(() => {
    flushSync()
    if (!document.querySelector('.preview .card')) throw new Error('not loaded')
  })
  return state
}

const button = (text: string, root: ParentNode = document) => [...root.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.trim() === text)!
const group = (label: string) => document.querySelector<HTMLElement>(`[role=radiogroup][aria-label="${label}"]`)!
const option = (groupLabel: string, label: string) =>
  [...group(groupLabel).querySelectorAll<HTMLButtonElement>('[role=radio]')].find((b) => (b.getAttribute('aria-label') ?? b.textContent?.trim()) === label)!
const previewCard = () => document.querySelector<HTMLElement>('.preview .card')!
const previewLine = () => document.querySelector<HTMLElement>('.preview .line')!
const previewName = () => previewLine().querySelector<HTMLElement>('.name')!
const click = (el: HTMLElement) => { el.click(); flushSync() }
/** Shows a tab (#467): only the picked tab's fields are shown (and can take focus). */
const showTab = (label: string) => click([...document.querySelectorAll<HTMLButtonElement>('[role=tab]')].find((t) => t.textContent!.trim() === label)!)
// a pick in a native colour picker: its input, then its change as it closes (input alone lands on the next frame)
const setColor = (el: HTMLInputElement, v: string) => {
  el.value = v
  el.dispatchEvent(new Event('input', { bubbles: true }))
  el.dispatchEvent(new Event('change', { bubbles: true }))
  flushSync()
}
const type = (el: HTMLInputElement, v: string) => { el.value = v; el.dispatchEvent(new Event('input', { bubbles: true })); flushSync() }
const bioBox = () => document.querySelector<HTMLElement>('[role=textbox][aria-label="Bio"]')!
function typeBio(t: string) {
  bioBox().textContent = t
  bioBox().dispatchEvent(new Event('input', { bubbles: true }))
  flushSync()
}

/** Pick a file in a section's Upload input, then Apply the crop. */
async function upload(section: 'Avatar' | 'Banner', file = new File([new Uint8Array([1, 2, 3])], 'a.png', { type: 'image/png' })) {
  const input = document.querySelector<HTMLInputElement>(`input[type=file][aria-label="Upload ${section.toLowerCase()}"]`)!
  expect(input.accept).toBe('image/png,image/jpeg,image/gif,image/webp')
  Object.defineProperty(input, 'files', { configurable: true, value: [file] })
  input.dispatchEvent(new Event('change', { bubbles: true }))
  flushSync()
  const img = await vi.waitFor(() => {
    const i = document.querySelector<HTMLImageElement>('[role=dialog][aria-label^="Crop"] img')
    if (!i) throw new Error('no crop image yet')
    return i
  })
  img.dispatchEvent(new Event('load'))
  flushSync()
  click(button('Apply', document.querySelector('[role=dialog][aria-label^="Crop"]')!))
}

beforeEach(() => {
  resetState()
  handlers = {}
  Object.defineProperty(HTMLImageElement.prototype, 'naturalWidth', { configurable: true, get: () => 800 })
  Object.defineProperty(HTMLImageElement.prototype, 'naturalHeight', { configurable: true, get: () => 600 })
  vi.mocked(invoke).mockReset()
  vi.mocked(invoke).mockImplementation((async (cmd: string, args: unknown, opts: unknown) => {
    const h = handlers[cmd]
    return h ? h(args, opts) : undefined
  }) as never)
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  vi.useRealTimers()
})

const badge = (key: string, rarity: Badge['rarity'], earned_at = '2026-11-01T00:00:00Z'): Badge =>
  ({ key, earned_at, first_earner: false, tier: 0, rarity, first_tiers: [], count: 1, hidden: false, seen: true }) as Badge
// jsdom has no PointerEvent: a MouseEvent with a pointerId stands in (as lib/railDrag.test.ts)
function pointer(type: string, el: Element, x: number) {
  const e = new MouseEvent(type, { bubbles: true, cancelable: true, clientX: x, button: 0 })
  Object.defineProperty(e, 'pointerId', { value: 1 })
  el.dispatchEvent(e)
}

describe('the redesigned editor (#467)', () => {
  const tabs = () => [...document.querySelectorAll<HTMLButtonElement>('[role=tablist][aria-label="Profile"] [role=tab]')]
  const panel = (label: string) => document.getElementById(tabs().find((t) => t.textContent!.trim() === label)!.getAttribute('aria-controls')!)!

  it('five tabs, About you first; only the picked one shows, the preview always does', async () => {
    await open()
    expect(tabs().map((t) => t.textContent!.trim())).toEqual(['About you', 'Card', 'Name style', 'Speaking ring', 'Badges'])
    expect(tabs()[0].getAttribute('aria-selected')).toBe('true')
    expect(panel('About you').hidden).toBe(false)
    for (const l of ['Card', 'Name style', 'Speaking ring', 'Badges']) expect(panel(l).hidden).toBe(true)
    // what each holds
    expect(panel('About you').querySelector('input[aria-label="Display name"]')).not.toBeNull()
    expect(panel('About you').querySelector('input[aria-label="Pronouns"]')).not.toBeNull()
    expect(panel('About you').querySelector('[role=textbox][aria-label="Bio"]')).not.toBeNull()
    expect(panel('About you').querySelector('input[aria-label="Upload avatar"]')).not.toBeNull()
    expect(panel('Card').querySelector('[role=radiogroup][aria-label="Card theme"]')).not.toBeNull()
    expect(panel('Card').querySelector('input[role=switch]')).not.toBeNull()
    expect(panel('Name style').querySelector('[aria-label="Name font"]')).not.toBeNull()
    expect(panel('Name style').querySelector('[aria-label="Name colour"]')).not.toBeNull()
    expect(panel('Name style').querySelector('[aria-label="Name effect"]')).not.toBeNull()
    showTab('Card')
    expect(panel('Card').hidden).toBe(false)
    expect(panel('About you').hidden).toBe(true)
    expect(previewCard()).not.toBeNull()
  })

  it('the arrow keys, Home and End move between the tabs', async () => {
    await open()
    tabs()[0].focus()
    tabs()[0].dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true, cancelable: true }))
    flushSync()
    expect(document.activeElement).toBe(tabs()[1])
    expect(tabs()[1].getAttribute('aria-selected')).toBe('true')
    tabs()[1].dispatchEvent(new KeyboardEvent('keydown', { key: 'End', bubbles: true, cancelable: true }))
    flushSync()
    expect(tabs()[4].getAttribute('aria-selected')).toBe('true')
    tabs()[4].dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true, cancelable: true }))
    flushSync()
    expect(tabs()[0].getAttribute('aria-selected')).toBe('true')
  })

  it('an edit on one tab is kept on another, and nothing is saved until Save', async () => {
    await open()
    type(document.querySelector<HTMLInputElement>('input[aria-label="Pronouns"]')!, 'she/her')
    showTab('Card')
    click(option('Card theme', 'Forest'))
    showTab('About you')
    expect(document.querySelector<HTMLInputElement>('input[aria-label="Pronouns"]')!.value).toBe('she/her')
    expect(calls('update_profile')).toHaveLength(0)
    expect(document.body.textContent).toContain('Unsaved changes')
  })

  it('the preview\'s avatar and banner offer to change them: a click opens that picture\'s picker', async () => {
    await open()
    const opened: string[] = []
    const spy = vi.spyOn(HTMLInputElement.prototype, 'click').mockImplementation(function (this: HTMLInputElement) { opened.push(this.getAttribute('aria-label')!) })
    try {
      const avatar = previewCard().querySelector<HTMLButtonElement>('button.edit-veil[aria-label="Change avatar"]')!
      const banner = previewCard().querySelector<HTMLButtonElement>('button.edit-veil[aria-label="Change banner"]')!
      expect(avatar.querySelector('svg')).not.toBeNull() // the pencil
      click(avatar)
      click(banner)
      expect(opened).toEqual(['Upload avatar', 'Upload banner'])
    } finally {
      spy.mockRestore()
    }
  })

  it('less text: the display name note is a tooltip, and the bio counter shows only near the limit', async () => {
    await open()
    const name = document.querySelector<HTMLInputElement>('input[aria-label="Display name"]')!
    expect(document.body.textContent).not.toContain('How your name shows everywhere')
    expect(name.dataset.tip).toContain('How your name shows everywhere')
    expect(document.querySelector('.counter')).toBeNull()
    typeBio('x'.repeat(150))
    expect(document.querySelector('.counter')!.textContent).toBe('150/190')
  })

  it('card themes are mini cards in their own colours', async () => {
    await open()
    const berry = option('Card theme', 'Berry')
    expect(berry.classList.contains('mini-card')).toBe(true)
    expect(berry.getAttribute('style')).toContain('--card-top: #b83c72')
    expect(berry.querySelector('.mc-avatar')).not.toBeNull()
    // Dusk (#444) is there too
    expect(option('Card theme', 'Dusk')).toBeDefined()
  })

  it('name colours sit in Solid and Gradient sections, then the custom +', async () => {
    await open()
    const groups = [...group('Name colour').querySelectorAll('.colour-group')]
    expect(groups.map((g) => g.querySelector('.group-label')!.textContent)).toEqual(['Solid', 'Gradient'])
    const labels = (g: Element) => [...g.querySelectorAll('[role=radio]')].map((r) => r.getAttribute('aria-label'))
    expect(labels(groups[0])).toEqual(['Default', 'Peach', 'Lilac', 'Mint', 'Sky'])
    expect(labels(groups[1])).toEqual(['Sunset gradient', 'Ice gradient', 'Mint gradient', 'Peach gradient', 'Aurora gradient', 'Custom'])
  })

  describe('Badges', () => {
    const held = [badge('a_common', 'common'), badge('b_rare', 'rare'), badge('c_epic', 'epic'), badge('d_legend', 'legendary')]
    const slots = () => [...document.querySelectorAll<HTMLElement>('.shown-badges [data-badge]')].map((b) => b.dataset.badge)
    const previewBadges = () => [...previewCard().querySelectorAll('.badges [aria-label]')].map((b) => b.getAttribute('aria-label'))
    const withRects = () => document.querySelectorAll<HTMLElement>('.shown-badges [data-badge]').forEach((b, i) => {
      b.getBoundingClientRect = () => ({ left: i * 60, right: i * 60 + 52, width: 52, top: 0, bottom: 52, height: 52, x: i * 60, y: 0, toJSON: () => ({}) })
    })

    it('with none earned, it says so', async () => {
      await open()
      expect(panel('Badges').textContent).toContain('No badges yet')
    })

    it('the rarest five by default; a drag reorders them, and the preview follows', async () => {
      await open(saved({ badges: held, showcase: ['d_legend', 'c_epic', 'b_rare', 'a_common'], showcase_chosen: false }))
      showTab('Badges')
      expect(slots()).toEqual(['d_legend', 'c_epic', 'b_rare', 'a_common'])
      expect(panel('Badges').textContent).toContain('Showing your rarest five')
      withRects()
      const first = document.querySelector<HTMLElement>('[data-badge="d_legend"]')!
      pointer('pointerdown', first, 20)
      pointer('pointermove', first, 150)
      pointer('pointerup', first, 150)
      flushSync()
      expect(slots()).toEqual(['c_epic', 'b_rare', 'd_legend', 'a_common'])
      expect(previewBadges()).toEqual(['C Epic', 'B Rare', 'D Legend', 'A Common'])
      expect(document.body.textContent).toContain('Unsaved changes')
    })

    it('Alt+arrows move the focused badge; clicking one takes it off; one from your badges goes on the end', async () => {
      await open(saved({ badges: held, showcase: ['a_common', 'b_rare'], showcase_chosen: true }))
      showTab('Badges')
      const b = document.querySelector<HTMLElement>('[data-badge="b_rare"]')!
      b.focus()
      b.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowLeft', altKey: true, bubbles: true, cancelable: true }))
      flushSync()
      expect(slots()).toEqual(['b_rare', 'a_common'])
      expect(document.activeElement).toBe(document.querySelector('[data-badge="b_rare"]'))
      click(document.querySelector<HTMLElement>('[data-badge="a_common"]')!)
      expect(slots()).toEqual(['b_rare'])
      click(document.querySelector<HTMLElement>('.all-badges [aria-label="D Legend: add to your card"]')!)
      expect(slots()).toEqual(['b_rare', 'd_legend'])
    })

    it('Save sends the order (and only the order, when nothing else changed); Rarest five sends none', async () => {
      handlers.get_profile = () => saved({ badges: held, showcase: ['a_common', 'b_rare'], showcase_chosen: true })
      await open(saved({ badges: held, showcase: ['a_common', 'b_rare'], showcase_chosen: true }))
      showTab('Badges')
      const b = document.querySelector<HTMLElement>('[data-badge="b_rare"]')!
      b.focus()
      b.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowLeft', altKey: true, bubbles: true, cancelable: true }))
      flushSync()
      handlers.get_profile = () => saved({ badges: held, showcase: ['b_rare', 'a_common'], showcase_chosen: true })
      click(button('Save'))
      await vi.waitFor(() => expect(calls('set_badge_showcase')).toHaveLength(1))
      expect(calls('set_badge_showcase')[0][1]).toEqual({ keys: ['b_rare', 'a_common'] })
      expect(calls('update_profile')).toHaveLength(0)
      await vi.waitFor(() => {
        flushSync()
        expect(document.body.textContent).not.toContain('Unsaved changes')
      })
      click(button('Rarest five'))
      expect(slots()).toEqual(['d_legend', 'c_epic', 'b_rare', 'a_common'])
      click(button('Save'))
      await vi.waitFor(() => expect(calls('set_badge_showcase')).toHaveLength(2))
      expect(calls('set_badge_showcase')[1][1]).toEqual({ keys: [] })
    })
  })
})

describe('speaking ring (#651)', () => {
  const tabs = () => [...document.querySelectorAll<HTMLButtonElement>('[role=tablist][aria-label="Profile"] [role=tab]')]
  const ringPanel = () => document.getElementById(tabs().find((t) => t.textContent!.trim() === 'Speaking ring')!.getAttribute('aria-controls')!)!
  const styles = () => [...group('Ring style').querySelectorAll<HTMLButtonElement>('[role=radio]')]
  const swatches = () => [...group('Ring colour').querySelectorAll<HTMLButtonElement>('[role=radio]')]
  const stage = () => document.querySelector<HTMLElement>('.ring-stage')!
  const ring = () => stage().querySelector<HTMLElement>('.ring-box')!
  const caption = () => document.querySelector<HTMLElement>('.ring-caption')!.textContent!.trim()
  afterEach(() => {
    Object.assign(motion, { reducedMotion: false, streaming: false })
    windowFocus.focused = true
  })

  it('the_speaking_ring_tab_follows_name_style', async () => {
    await open()
    expect(tabs().map((t) => t.textContent!.trim())).toEqual(['About you', 'Card', 'Name style', 'Speaking ring', 'Badges'])
    // the tab keys step through it like the others
    const name = tabs()[2]
    showTab('Name style')
    name.focus()
    name.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true, cancelable: true }))
    flushSync()
    expect(document.activeElement).toBe(tabs()[3])
    expect(ringPanel().hidden).toBe(false)
    expect(ringPanel().querySelector('h3')!.textContent!.trim()).toBe('Speaking ring')
    expect(ringPanel().textContent).toContain('How your avatar lights up when you talk. Everyone in the call sees it.')
    expect(ringPanel().textContent).toContain('With reduced motion on, everyone sees the still version of each ring. In big calls some rings show still to keep things smooth.')
    tabs()[3].dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true, cancelable: true }))
    flushSync()
    expect(document.activeElement!.textContent!.trim()).toBe('Badges')
  })

  it('style_and_colour_are_radiogroups_with_roving_tabindex', async () => {
    await open()
    showTab('Speaking ring')
    expect(styles()).toHaveLength(11)
    const groups = [...group('Ring style').querySelectorAll<HTMLElement>('[role=group]')]
    expect(groups.map((g) => document.getElementById(g.getAttribute('aria-labelledby')!)!.textContent!.trim())).toEqual(['Still or looping', 'Follows your voice'])
    expect(groups.map((g) => g.querySelectorAll('[role=radio]').length)).toEqual([8, 3])
    expect(styles().filter((r) => r.tabIndex === 0)).toEqual([option('Ring style', 'Classic')])
    expect(option('Ring style', 'Classic').getAttribute('aria-checked')).toBe('true')
    // the arrows cross from one group into the next, and pick
    click(option('Ring style', 'Viewfinder'))
    const vf = option('Ring style', 'Viewfinder')
    vf.focus()
    vf.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true, cancelable: true }))
    flushSync()
    const liquid = option('Ring style', 'Liquid')
    expect(document.activeElement).toBe(liquid)
    expect(liquid.getAttribute('aria-checked')).toBe('true')
    expect(styles().filter((r) => r.tabIndex === 0)).toEqual([liquid])
    // the preview draws the draft
    expect(ring().dataset.style).toBe('liquid')
    // colours: the nine, by name, one tab stop
    expect(swatches().map((b) => b.getAttribute('aria-label'))).toEqual(RING_COLORS.map((c) => c.label))
    expect(swatches().filter((r) => r.tabIndex === 0)).toEqual([option('Ring colour', 'Green')])
    expect(option('Ring colour', 'Pulse blue').style.getPropertyValue('--swatch')).toBe('var(--speak-blue)')
    click(option('Ring colour', 'Cyan'))
    expect(ring().style.getPropertyValue('--speak')).toBe('var(--speak-cyan)')
  })

  it('the_profile_swatch_tip_says_where_the_colour_comes_from', async () => {
    const coloured = { ...alex, style: { font: null, colors: ['#ffbe7a'] } }
    await open(saved({ user: coloured }))
    showTab('Speaking ring')
    const prof = option('Ring colour', 'Profile colour')
    expect(prof.dataset.tip).toBe('Your name colour, from Name style')
    const want = speakCss(previewUser(alex, draftFrom(saved({ user: coloured }))), 'profile', nameBg('side'))
    expect(prof.style.getPropertyValue('--swatch')).toBe(want)
    if (cmp) unmount(cmp)
    cmp = undefined
    document.body.innerHTML = ''
    await open()
    showTab('Speaking ring')
    expect(option('Ring colour', 'Profile colour').dataset.tip).toMatch(/ You haven't set one, so it's green\.$/)
    expect(option('Ring colour', 'Profile colour').style.getPropertyValue('--swatch')).toBe('var(--speak-green)')
  })

  it('picking_a_ring_saves_it', async () => {
    const state = await open()
    showTab('Speaking ring')
    click(option('Ring style', 'Bars round the ring'))
    click(option('Ring colour', 'Purple'))
    expect(state.dirty).toBe(true)
    click(button('Save'))
    expect(calls('update_profile')).toHaveLength(1)
    const req = (calls('update_profile')[0][1] as { request: UpdateProfileRequest }).request
    expect(req.ring_style).toBe('bars')
    expect(req.ring_color).toBe('purple')
  })

  it('a_saved_ring_is_what_the_tab_shows', async () => {
    await open(saved({ user: { ...alex, ring_style: 'dots', ring_color: 'gold' } }))
    showTab('Speaking ring')
    expect(option('Ring style', 'Wave of dots').getAttribute('aria-checked')).toBe('true')
    expect(option('Ring colour', 'Gold').getAttribute('aria-checked')).toBe('true')
    expect(ring().dataset.style).toBe('dots')
  })

  it('the_preview_animates_only_while_the_tab_is_shown', async () => {
    await open(saved({ user: { ...alex, ring_style: 'bars' } }))
    vi.useFakeTimers()
    const baseline = vi.getTimerCount()
    showTab('Speaking ring')
    FakeIntersectionObserver.set(stage(), true)
    flushSync()
    expect(ring().dataset.ring).toBe('on')
    expect(caption()).toBe('Preview · talking')
    vi.advanceTimersByTime(70)
    expect(stage().style.getPropertyValue('--lvl')).not.toBe('')
    expect(vi.getTimerCount()).toBeGreaterThan(baseline)
    showTab('Badges')
    // one-shot timeouts (a tab switch may start one) run out; a repeating interval would not
    vi.advanceTimersByTime(1000)
    expect(vi.getTimerCount()).toBe(baseline)
    expect(stage().style.getPropertyValue('--lvl')).toBe('')
    // reduced motion: the still version, said so, and no envelope at all
    motion.reducedMotion = true
    showTab('Speaking ring')
    expect(ring().dataset.ring).toBe('still')
    expect(caption()).toBe('Preview · still (reduced motion is on)')
    vi.advanceTimersByTime(1000)
    expect(vi.getTimerCount()).toBe(baseline)
    expect(stage().style.getPropertyValue('--lvl')).toBe('')
  })

  it('the_preview_is_still_while_you_are_live', async () => {
    await open(saved({ user: { ...alex, ring_style: 'bars' } }))
    vi.useFakeTimers()
    const baseline = vi.getTimerCount()
    motion.streaming = true
    showTab('Speaking ring')
    FakeIntersectionObserver.set(stage(), true)
    flushSync()
    // drawn still, so the caption says still, and no envelope runs
    expect(ring().dataset.ring).toBe('still')
    expect(caption()).toBe("Preview · still (you're live)")
    vi.advanceTimersByTime(1000)
    expect(vi.getTimerCount()).toBe(baseline)
    expect(stage().style.getPropertyValue('--lvl')).toBe('')
    // going off air: it talks again
    motion.streaming = false
    flushSync()
    expect(caption()).toBe('Preview · talking')
    vi.advanceTimersByTime(70)
    expect(vi.getTimerCount()).toBeGreaterThan(baseline)
  })

  it('the_envelope_stops_while_the_window_is_unfocused_or_the_preview_off_screen', async () => {
    await open(saved({ user: { ...alex, ring_style: 'dots' } }))
    vi.useFakeTimers()
    const baseline = vi.getTimerCount()
    showTab('Speaking ring')
    FakeIntersectionObserver.set(stage(), true)
    flushSync()
    vi.advanceTimersByTime(70)
    expect(vi.getTimerCount()).toBeGreaterThan(baseline)
    // the window loses focus (minimised, another app on top): the same signal animated emoji follow
    windowFocus.focused = false
    flushSync()
    vi.advanceTimersByTime(1000)
    expect(vi.getTimerCount()).toBe(baseline)
    windowFocus.focused = true
    flushSync()
    expect(vi.getTimerCount()).toBeGreaterThan(baseline)
    // scrolled out of view
    FakeIntersectionObserver.set(stage(), false)
    flushSync()
    vi.advanceTimersByTime(1000)
    expect(vi.getTimerCount()).toBe(baseline)
  })

  describe('style chip tooltips', () => {
    // jsdom lays nothing out: a chip "overflows" when its label is longer than 12 characters
    const wide = (b: Element) => (b.textContent!.trim().length > 12 ? 160 : 60)
    beforeEach(() => {
      Object.defineProperty(HTMLButtonElement.prototype, 'scrollWidth', { configurable: true, get() { return wide(this) } })
      Object.defineProperty(HTMLButtonElement.prototype, 'clientWidth', { configurable: true, get() { return 100 } })
    })
    afterEach(() => {
      delete (HTMLButtonElement.prototype as { scrollWidth?: number }).scrollWidth
      delete (HTMLButtonElement.prototype as { clientWidth?: number }).clientWidth
      vi.unstubAllGlobals()
      vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
      vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)
    })

    it('a_chip_tip_shows_only_when_its_label_is_cut_off', async () => {
      await open()
      showTab('Speaking ring')
      expect(option('Ring style', 'Bars round the ring').dataset.tip).toBe('Bars round the ring')
      expect(option('Ring style', 'Classic').dataset.tip).toBeUndefined()
      expect(option('Ring style', 'Wave of dots').dataset.tip).toBeUndefined()
    })

    it('a_resize_rechecks_the_chips', async () => {
      const observers: { cb: () => void; observed: Element[] }[] = []
      vi.stubGlobal('ResizeObserver', class {
        observed: Element[] = []
        constructor(public cb: () => void) { observers.push(this) }
        observe(el: Element) { this.observed.push(el) }
        unobserve() {}
        disconnect() { this.observed = [] }
      })
      await open()
      showTab('Speaking ring')
      const grids = [...document.querySelectorAll('.ring-grid')]
      // one observer per grid
      expect(observers.filter((o) => o.observed.some((e) => grids.includes(e)))).toHaveLength(2)
      expect(option('Ring style', 'Bars round the ring').dataset.tip).toBe('Bars round the ring')
      // the pane gets wider: everything fits, no tips
      Object.defineProperty(HTMLButtonElement.prototype, 'clientWidth', { configurable: true, get() { return 200 } })
      for (const o of observers) o.cb()
      expect(option('Ring style', 'Bars round the ring').dataset.tip).toBeUndefined()
      // narrower again: Classic is cut off too
      Object.defineProperty(HTMLButtonElement.prototype, 'clientWidth', { configurable: true, get() { return 40 } })
      for (const o of observers) o.cb()
      expect(option('Ring style', 'Classic').dataset.tip).toBe('Classic')
    })
  })
})

describe('ProfileSettings', () => {
  it('presets update the preview', async () => {
    const state = await open()
    expect(calls('get_profile')).toHaveLength(1)
    expect(previewCard().getAttribute('style')).toContain('--glow-top: #3a3d46')
    expect(option('Card theme', 'Plain').getAttribute('aria-checked')).toBe('true')
    expect(state.dirty).toBe(false)
    click(option('Card theme', 'Berry'))
    expect(previewCard().getAttribute('style')).toContain('--glow-top: #c2457a')
    expect(previewCard().getAttribute('style')).toContain('--glow-bottom: #3a1f5e')
    expect(option('Card theme', 'Berry').getAttribute('aria-checked')).toBe('true')
    expect(state.dirty).toBe(true)
    expect(document.body.textContent).toContain('Unsaved changes')
    click(option('Name colour', 'Mint'))
    expect(previewName().getAttribute('style')).toContain('color: rgb(168, 230, 207)')
  })

  it('custom pickers update the preview and note lightening', async () => {
    await open()
    click(option('Card theme', 'Custom'))
    const [top, bottom] = document.querySelectorAll<HTMLInputElement>('.theme input[type=color]')
    setColor(top, '#ffffff')
    setColor(bottom, '#ffff00')
    expect(previewCard().getAttribute('style')).toContain('--glow-top: #ffffff')
    expect(previewCard().getAttribute('style')).toContain('--glow-bottom: #ffff00')
    expect(option('Card theme', 'Custom').getAttribute('aria-checked')).toBe('true')

    // the note is a tooltip on the wells now (#467: explanations become tooltips)
    const note = 'Lightened so it reads on dark backgrounds'
    const tips = () => [...document.querySelectorAll<HTMLInputElement>('.colour input[type=color]')].map((i) => i.dataset.tip)
    click(option('Name colour', 'Custom'))
    expect(tips()).toEqual([undefined])
    const pick = document.querySelector<HTMLInputElement>('.colour input[type=color]')!
    setColor(pick, '#000000')
    expect(previewName().getAttribute('style')).toContain('color: rgb(168, 168, 168)')
    expect(tips()).toEqual([note])
    click(option('Custom colour', 'Gradient'))
    const pickers = document.querySelectorAll<HTMLInputElement>('.colour input[type=color]')
    expect(pickers).toHaveLength(2)
    setColor(pickers[1], '#0000ff')
    expect(previewName().getAttribute('style')).toContain('linear-gradient(90deg, rgb(168, 168, 168), rgb(161, 161, 255))')
    setColor(pickers[0], '#ffc4a3')
    setColor(pickers[1], '#a3d8ff')
    expect(tips()).toEqual([undefined, undefined])
  })

  it('picking a font and gradient restyles the preview name', async () => {
    await open()
    const fonts = [...group('Name font').querySelectorAll<HTMLButtonElement>('[role=radio]')]
    expect(fonts.map((b) => b.getAttribute('aria-label'))).toEqual(['Default', 'Elegant', 'Rounded', 'Pixel', 'Script', 'Handwritten', 'Grotesque', 'Quirky', 'Techy', 'Bold', '8-bit', 'Racing', 'Retro', 'Brush', 'Stencil'])
    // each tile is your own name in that face (#467)
    expect(fonts.every((b) => b.textContent!.trim() === 'alex')).toBe(true)
    expect(fonts[3].querySelector('.name')!.getAttribute('style')).toContain('Press Start 2P')
    click(option('Name font', 'Techy'))
    click(option('Name colour', 'Aurora gradient'))
    const css = previewName().getAttribute('style')!
    expect(css).toContain('Orbitron')
    expect(css).toContain('linear-gradient(90deg, rgb(246, 182, 255), rgb(159, 180, 255))')
    expect(previewCard().querySelector('.name')!.getAttribute('style')).toContain('Orbitron')
    expect(previewLine().textContent).toContain('Looking good!')
    // nothing outside the editor changed
    expect(app.state.me!.style).toEqual({ font: null, colors: [] })
  })

  it('the name effect (#468): None and the eight, each on your own name style; a pick plays in the preview and saves', async () => {
    await open()
    const opts = [...group('Name effect').querySelectorAll<HTMLButtonElement>('[role=radio]')]
    expect(opts.map((b) => b.getAttribute('aria-label') ?? b.textContent!.trim())).toEqual(['None', 'Neon', 'Toon', 'Pop', 'Gummy', 'Prism', 'Shimmer', 'Glitch', 'Ember'])
    expect(option('Name effect', 'None').getAttribute('aria-checked')).toBe('true')
    // each chip draws the effect, still until hovered
    const neon = option('Name effect', 'Neon')
    expect(neon.querySelector<HTMLElement>('.name')!.dataset.fx).toBe('neon')
    expect(neon.querySelector('.name')!.classList.contains('fx-play')).toBe(false)
    neon.dispatchEvent(new Event('pointerenter'))
    flushSync()
    expect(neon.querySelector('.name')!.classList.contains('fx-play')).toBe(true)
    expect(neon.getAttribute('data-tip')).toBe('Glow pulses')
    click(option('Name effect', 'Ember'))
    // the preview plays it, in the line and on the card
    expect(previewName().dataset.fx).toBe('ember')
    expect(previewName().classList.contains('fx-play')).toBe(true)
    expect(previewCard().querySelector<HTMLElement>('.name')!.classList.contains('fx-play')).toBe(true)
    click(button('Save'))
    const req = (calls('update_profile')[0][1] as { request: UpdateProfileRequest }).request
    expect(req.style.effect).toBe('ember')
  })

  it('upload goes through crop then stages the hash without saving', async () => {
    let finish!: (m: MediaInfo) => void
    handlers.upload_avatar_media = () => new Promise<MediaInfo>((r) => (finish = r))
    const state = await open()
    await upload('Avatar')
    expect(document.querySelector('[role=dialog][aria-label^="Crop"]')).toBeNull()
    await vi.waitFor(() => {
      flushSync()
      expect(calls('upload_avatar_media')).toHaveLength(1)
    })
    const [, bytes, opts] = calls('upload_avatar_media')[0] as unknown as [string, Uint8Array, { headers: Record<string, string> }]
    expect([...bytes]).toEqual([1, 2, 3])
    expect(opts.headers['x-pulse-crop']).toBe('100,0,600,600')
    expect(document.body.textContent).toContain('Uploading…')
    finish(media('AV', 'AVS'))
    await vi.waitFor(() => {
      flushSync()
      expect(document.body.textContent).not.toContain('Uploading…')
    })
    // the preview's chat line and card show the staged still; the store doesn't
    expect(previewLine().querySelector('img')!.getAttribute('src')).toContain('AVS')
    expect(previewCard().querySelector('.ring img')!.getAttribute('src')).toContain('AVS')
    expect(state.dirty).toBe(true)
    expect(calls('update_profile')).toHaveLength(0)
    expect(app.state.me!.avatar_hash).toBeNull()

    handlers.upload_banner_media = () => media('BN')
    await upload('Banner')
    await vi.waitFor(() => {
      flushSync()
      expect(previewCard().querySelector('.banner img')?.getAttribute('src')).toContain('BN')
    })
    click(button('Remove', document.querySelector('.banner-field')!))
    expect(previewCard().querySelector('.banner img')).toBeNull()
    expect(calls('update_profile')).toHaveLength(0)
  })

  it('a refused upload keeps the draft and shows the message', async () => {
    await open(saved({ user: { ...alex, avatar_hash: 'OLD' } }))
    const refusals = [
      'that file is too big (8 MB at most)',
      'that animation has too many frames (200 at most)',
      "that image's dimensions are too large (4096×4096 at most)",
      "that isn't a supported image: use PNG, JPEG, GIF or WebP",
      'The crop is outside the image.',
    ]
    for (const msg of refusals) {
      handlers.upload_avatar_media = () => Promise.reject(msg)
      await upload('Avatar')
      await vi.waitFor(() => {
        flushSync()
        expect(document.querySelector('.avatar-field [role=alert]')?.textContent).toBe(msg)
      })
      expect(previewLine().querySelector('img')!.getAttribute('src')).toContain('OLD')
      expect(document.body.textContent).not.toContain('Unsaved changes')
    }
    expect(calls('update_profile')).toHaveLength(0)
  })

  it('Save sends the whole profile once', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true })
    let answer!: (p: Profile) => void
    handlers.update_profile = () => new Promise<Profile>((r) => (answer = r))
    const state = await open()
    click(option('Card theme', 'Forest'))
    click(option('Name font', 'Rounded'))
    click(option('Name colour', 'Custom'))
    setColor(document.querySelector<HTMLInputElement>('.colour input[type=color]')!, '#000000')
    type(document.querySelector<HTMLInputElement>('input[aria-label="Pronouns"]')!, '')
    click(button('Save'))
    click(button('Save')) // a second click while saving sends nothing
    expect(calls('update_profile')).toHaveLength(1)
    const req = (calls('update_profile')[0][1] as { request: UpdateProfileRequest }).request
    expect(req).toEqual({
      avatar_hash: null, banner_hash: null, theme_top: '#1f7a5a', theme_bottom: '#0f2b3a',
      style: { font: 'fredoka', colors: ['#000000'], effect: null }, pronouns: null, bio: 'hello', display_name: 'alex', glow: true, typing_text: '',
      ring_style: 'classic', ring_color: 'green',
    })
    const stored = saved({
      user: { ...alex, style: { font: 'fredoka', colors: ['#a8a8a8'] } },
      card: { ...saved().card, theme_top: '#1f7a5a', theme_bottom: '#0f2b3a', pronouns: null },
    })
    answer(stored)
    await vi.waitFor(() => {
      flushSync()
      expect(document.body.textContent).toContain('Saved')
    })
    expect(state.dirty).toBe(false)
    expect(document.body.textContent).not.toContain('Unsaved changes')
    // the draft took the server's answer (the lightened colour), and the cache has it
    expect(document.querySelector<HTMLInputElement>('.colour input[type=color]')!.value).toBe('#a8a8a8')
    expect(app.state.profiles.U1).toEqual(stored)
    await vi.advanceTimersByTimeAsync(2000)
    flushSync()
    expect(document.querySelector('.footer')!.textContent).not.toContain('Saved')

    // a refused Save keeps the draft and shows the message
    handlers.update_profile = () => Promise.reject('Pronouns can be up to 40 characters.')
    click(option('Card theme', 'Berry'))
    click(button('Save'))
    await vi.waitFor(() => {
      flushSync()
      expect(document.querySelector('.footer [role=alert]')?.textContent).toBe('Pronouns can be up to 40 characters.')
    })
    expect(option('Card theme', 'Berry').getAttribute('aria-checked')).toBe('true')
    expect(state.dirty).toBe(true)
  })

  describe('name colour Default and Custom (#165)', () => {
    it('Default is a swatch-style "none" chip and Custom a labelled icon button, neither with text', async () => {
      await open()
      const def = option('Name colour', 'Default')
      const custom = option('Name colour', 'Custom')
      expect(def.classList.contains('swatch')).toBe(true)
      expect(def.classList.contains('none')).toBe(true)
      expect(def.textContent!.trim()).toBe('')
      expect(custom.classList.contains('swatch')).toBe(true)
      expect(custom.querySelector('svg')).not.toBeNull()
      expect(custom.textContent!.trim()).toBe('')
      expect(custom.dataset.tip).toBe('Custom colour')
    })

    it('clicking Custom opens the colour picker', async () => {
      await open()
      const opened: HTMLInputElement[] = []
      const spy = vi.spyOn(HTMLInputElement.prototype, 'click').mockImplementation(function (this: HTMLInputElement) { opened.push(this) })
      try {
        click(option('Name colour', 'Custom'))
        expect(opened).toEqual([document.querySelector('.colour input[type=color]')])
      } finally {
        spy.mockRestore()
      }
    })

    it('arrowing onto Custom selects it without popping the picker open', async () => {
      await open()
      const opened: HTMLInputElement[] = []
      const spy = vi.spyOn(HTMLInputElement.prototype, 'click').mockImplementation(function (this: HTMLInputElement) { opened.push(this) })
      try {
        showTab('Name style')
        const def = option('Name colour', 'Default')
        def.focus()
        def.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowLeft', bubbles: true, cancelable: true }))
        flushSync()
        expect(option('Name colour', 'Custom').getAttribute('aria-checked')).toBe('true')
        expect(opened).toEqual([])
      } finally {
        spy.mockRestore()
      }
    })
  })

  describe('typing text (#652)', () => {
    const box = () => document.querySelector<HTMLInputElement>('input[aria-label="Typing text"]')!
    const field = () => document.querySelector<HTMLElement>('.typing-field')!
    const preview = () => field().querySelector<HTMLElement>('.typing-preview')!.textContent!.replace(/\s+/g, ' ').trim()
    const suggestions = () => [...field().querySelectorAll<HTMLButtonElement>('.typing-suggestions button')]

    it('reads "<your name> is [ … ]": the name and a fixed "is" before the box, which holds only what follows', async () => {
      await open()
      const row = field().querySelector<HTMLElement>('.typing-row')!
      expect(row.querySelector('.typing-who')!.textContent).toBe('alex')
      expect(row.querySelector('.typing-is')!.textContent).toBe('is')
      expect(row.lastElementChild).toBe(box())
      expect(box().value).toBe('')
      expect(box().placeholder).toBe('typing…')
      // your name as the draft has it
      type(document.querySelector<HTMLInputElement>('input[aria-label="Display name"]')!, 'Alex 🎸')
      expect(row.querySelector('.typing-who')!.textContent).toBe('Alex 🎸')
    })

    it('a live preview: blank is the plain "is typing…", anything else is your words', async () => {
      await open()
      expect(preview()).toBe('alex is typing…')
      type(box(), 'plotting')
      expect(preview()).toBe('alex is plotting')
      type(box(), '   ')
      expect(preview()).toBe('alex is typing…')
    })

    it('six suggestions under the box: a tap fills it, and the picked one shows as pressed', async () => {
      await open()
      expect(suggestions().map((b) => b.textContent!.trim())).toEqual(['cooking', 'yapping', 'plotting', 'barking', 'typing a novel', 'scheming'])
      click(suggestions()[3])
      expect(box().value).toBe('barking')
      expect(preview()).toBe('alex is barking')
      expect(suggestions()[3].getAttribute('aria-pressed')).toBe('true')
      expect(suggestions()[0].getAttribute('aria-pressed')).toBe('false')
    })

    it('over 32 characters, a link or a line-break trick: says so and Save is off', async () => {
      await open()
      type(box(), 'x'.repeat(33))
      expect(field().querySelector('[role=alert]')!.textContent).toBe('Typing text can be up to 32 characters.')
      expect(button('Save').disabled).toBe(true)
      type(box(), 'at pulse.app')
      expect(field().querySelector('[role=alert]')!.textContent).toBe("Typing text can't contain links.")
      type(box(), 'x'.repeat(32))
      expect(field().querySelector('[role=alert]')).toBeNull()
      expect(button('Save').disabled).toBe(false)
    })

    it('saves with the profile, and a saved one is what the editor shows', async () => {
      handlers.update_profile = () => saved({ user: { ...alex, typing_text: 'typing a novel' } })
      await open(saved({ user: { ...alex, typing_text: 'cooking' } }))
      expect(box().value).toBe('cooking')
      expect(preview()).toBe('alex is cooking')
      type(box(), ' typing a novel ')
      click(button('Save'))
      expect((calls('update_profile')[0][1] as { request: UpdateProfileRequest }).request.typing_text).toBe('typing a novel')
    })

    it('fits a narrow window: the row shrinks (the box first), a long name and the preview end in an ellipsis', async () => {
      const { compile } = await import('svelte/compiler')
      const { readFileSync } = await import('node:fs')
      const { resolve } = await import('node:path')
      const css = compile(readFileSync(resolve(__dirname, 'ProfileSettings.svelte'), 'utf8'), { filename: 'ProfileSettings.svelte', css: 'external', cssHash: () => 's' }).css!.code
      const rule = (sel: string) => css.match(new RegExp(`(?:^|\\n|\\})\\s*${sel.split(' ').map((p) => `${p.replace(/\./g, '\\.')}\\.s`).join(' ')}\\s*\\{([^}]*)\\}`))?.[1] ?? ''
      expect(rule('.typing-row')).toMatch(/display:\s*flex/)
      expect(rule('.typing-row')).toMatch(/min-width:\s*0/)
      expect(css.match(/\.typing-row\.s \.input:where\(\.s\) \{([^}]*)\}/)?.[1]).toMatch(/flex:\s*1 1 0/)
      expect(css.match(/\.typing-row\.s \.input:where\(\.s\) \{([^}]*)\}/)?.[1]).toMatch(/min-width:\s*0/)
      for (const sel of ['.typing-who', '.typing-preview']) {
        expect(rule(sel), sel).toMatch(/overflow:\s*hidden/)
        expect(rule(sel), sel).toMatch(/text-overflow:\s*ellipsis/)
        expect(rule(sel), sel).toMatch(/white-space:\s*nowrap/)
      }
      expect(rule('.typing-suggestions')).toMatch(/flex-wrap:\s*wrap/)
    })
  })

  describe('display name (#47)', () => {
    const nameBox = () => document.querySelector<HTMLInputElement>('input[aria-label="Display name"]')!
    beforeEach(() => { try { localStorage.clear() } catch { /* none */ } })

    it('previews in your name style and saves with the profile', async () => {
      handlers.update_profile = (a) => saved({ user: { ...alex, display_name: (a as { request: UpdateProfileRequest }).request.display_name! } })
      await open()
      expect(nameBox().value).toBe('alex')
      type(nameBox(), 'Alex 🎸 A')
      expect(previewName().textContent).toBe('Alex 🎸 A')
      expect(previewCard().querySelector('.name')!.textContent).toBe('Alex 🎸 A')
      expect(previewCard().querySelector('.handle')!.textContent).toContain('@alex')
      click(button('Save'))
      expect((calls('update_profile')[0][1] as { request: UpdateProfileRequest }).request.display_name).toBe('Alex 🎸 A')
    })

    it('blank, too long or with a line break: says so and Save is off', async () => {
      await open()
      for (const bad of ['   ', 'x'.repeat(33)]) {
        type(nameBox(), bad)
        expect(document.querySelector('.name-field [role=alert]')!.textContent).toContain('1 to 32')
        expect(button('Save').disabled).toBe(true)
      }
      type(nameBox(), 'x'.repeat(32))
      expect(document.querySelector('.name-field [role=alert]')).toBeNull()
      expect(button('Save').disabled).toBe(false)
    })

    it('a Zalgo name gets a soft "Zalgo? Really?" and still saves (#209)', async () => {
      await open()
      const zalgo = 'Z\u{335}\u{352}\u{316}alex'
      type(nameBox(), zalgo)
      const hint = document.querySelector('.name-field .zalgo')!
      expect(hint.textContent).toBe('Zalgo? Really?')
      expect(hint.getAttribute('role')).not.toBe('alert')
      expect(document.querySelector('.name-field [role=alert]')).toBeNull()
      expect(nameBox().getAttribute('aria-invalid')).toBe('false')
      expect(button('Save').disabled).toBe(false)
      click(button('Save'))
      expect((calls('update_profile')[0][1] as { request: UpdateProfileRequest }).request.display_name).toBe(zalgo)
      type(nameBox(), 'alex')
      expect(document.querySelector('.name-field .zalgo')).toBeNull()
    })

    it('nudges once while your name is still your username, until dismissed', async () => {
      await open()
      expect(document.querySelector('.nudge')!.textContent).toContain('Pick how your name looks')
      click(button('Dismiss'))
      expect(document.querySelector('.nudge')).toBeNull()
      unmount(cmp!)
      cmp = undefined
      await open()
      expect(document.querySelector('.nudge')).toBeNull()
    })

    it('no nudge once you have a display name of your own', async () => {
      await open(saved({ user: { ...alex, display_name: 'Alex A' } }))
      expect(document.querySelector('.nudge')).toBeNull()
    })
  })

  it('a drag in a colour picker (a burst of input events) redraws the preview once a frame, with the last colour (#156 follow-up)', async () => {
    await open()
    click(option('Card theme', 'Custom'))
    const [top] = document.querySelectorAll<HTMLInputElement>('.theme input[type=color]')
    const card = previewCard()
    let redraws = 0
    const seen = new MutationObserver((m) => (redraws += m.length))
    seen.observe(card, { attributes: true, attributeFilter: ['style'] })
    for (const c of ['#101010', '#202020', '#303030', '#404040', '#505050', '#ffb6d9']) {
      top.value = c
      top.dispatchEvent(new Event('input', { bubbles: true }))
      flushSync()
    }
    await Promise.resolve()
    expect(redraws).toBe(0)
    await new Promise((r) => requestAnimationFrame(r))
    flushSync()
    await Promise.resolve()
    expect(redraws).toBe(1)
    expect(card.getAttribute('style')).toContain('--glow-top: #ffb6d9')
    seen.disconnect()
  })

  it('Save takes a colour still waiting for its frame', async () => {
    await open()
    click(option('Card theme', 'Custom'))
    const [top] = document.querySelectorAll<HTMLInputElement>('.theme input[type=color]')
    setColor(top, '#202020')
    top.value = '#ffb6d9'
    top.dispatchEvent(new Event('input', { bubbles: true }))
    click(button('Save'))
    const req = (calls('update_profile')[0][1] as { request: UpdateProfileRequest }).request
    expect(req.theme_top).toBe('#ffb6d9')
  })

  it('the glow switch (#156): on by default, saved with the profile, the preview follows it', async () => {
    const state = await open()
    const glow = document.querySelector<HTMLInputElement>('input[role=switch]')!
    expect(glow.closest('label')!.textContent!.trim()).toBe('Glow')
    // what it does is a tooltip (#467)
    expect(glow.closest('label')!.dataset.tip).toContain('soft glow')
    expect(glow.checked).toBe(true)
    expect(previewCard().classList.contains('glow')).toBe(true)
    click(glow)
    expect(previewCard().classList.contains('glow')).toBe(false)
    expect(state.dirty).toBe(true)
    click(button('Save'))
    const req = (calls('update_profile')[0][1] as { request: UpdateProfileRequest }).request
    expect(req.glow).toBe(false)
  })

  it('a saved glow off is what the editor shows', async () => {
    await open(saved({ card: { ...saved().card, glow: false } }))
    expect(document.querySelector<HTMLInputElement>('input[role=switch]')!.checked).toBe(false)
    expect(previewCard().classList.contains('glow')).toBe(false)
  })

  it('the custom theme shows its colours: on the Custom chip and as hex beside each well (#141)', async () => {
    await open(saved({ card: { ...saved().card, theme_top: '#ffb6d9', theme_bottom: '#ffffff' } }))
    // the + tile is a mini card in the custom colours (#467), and opens their pickers in a popover
    const tile = option('Card theme', 'Custom')
    expect(tile.getAttribute('style')).toContain('--card-top: #ffb6d9')
    expect(tile.querySelector('.mc-banner')).not.toBeNull()
    expect(document.querySelector('[role=dialog][aria-label="Custom card colours"]')).toBeNull()
    click(tile)
    const labels = [...document.querySelectorAll('.theme .pick')].map((l) => l.textContent!.replace(/\s+/g, ' ').trim())
    expect(labels).toEqual(['Top #ffb6d9', 'Bottom #ffffff'])
    // a preset picked: the + shows no colours
    click(option('Card theme', 'Night sky'))
    expect(option('Card theme', 'Custom').querySelector('.mc-banner')).toBeNull()
    expect(document.querySelector('[role=dialog][aria-label="Custom card colours"]')).toBeNull()
  })

  it('a saved custom card theme is what the editor shows after signing out and in (#139)', async () => {
    handlers.update_profile = (args) => {
      const r = (args as { request: UpdateProfileRequest }).request
      return saved({ card: { ...saved().card, theme_top: r.theme_top, theme_bottom: r.theme_bottom } })
    }
    await open()
    click(option('Card theme', 'Custom'))
    const [top, bottom] = document.querySelectorAll<HTMLInputElement>('.theme input[type=color]')
    setColor(top, '#ffb6d9')
    setColor(bottom, '#ffffff')
    click(button('Save'))
    await vi.waitFor(() => expect(app.state.profiles.U1?.card.theme_top).toBe('#ffb6d9'))
    // sign out, sign in, open Settings → Profile again
    unmount(cmp!)
    cmp = undefined
    document.body.innerHTML = ''
    resetState()
    await open(saved({ card: { ...saved().card, theme_top: '#ffb6d9', theme_bottom: '#ffffff' } }))
    expect(option('Card theme', 'Custom').getAttribute('aria-checked')).toBe('true')
    click(option('Card theme', 'Custom'))
    const [t2, b2] = document.querySelectorAll<HTMLInputElement>('.theme input[type=color]')
    expect([t2?.value, b2?.value]).toEqual(['#ffb6d9', '#ffffff'])
    expect(previewCard().getAttribute('style')).toContain('--glow-top: #ffb6d9')
  })

  it('Reset restores the saved profile', async () => {
    const state = await open()
    click(option('Card theme', 'Sunset'))
    click(option('Name font', 'Pixel'))
    type(document.querySelector<HTMLInputElement>('input[aria-label="Pronouns"]')!, 'she/her')
    typeBio('changed')
    expect(state.dirty).toBe(true)
    click(button('Reset'))
    expect(state.dirty).toBe(false)
    expect(option('Card theme', 'Plain').getAttribute('aria-checked')).toBe('true')
    expect(option('Name font', 'Default').getAttribute('aria-checked')).toBe('true')
    expect(document.querySelector<HTMLInputElement>('input[aria-label="Pronouns"]')!.value).toBe('they/them')
    expect(bioBox().textContent).toBe('hello')
    expect(button('Reset').disabled).toBe(true)
    expect(button('Save').disabled).toBe(true)
  })

  it('bio over 190 disables Save', async () => {
    await open()
    typeBio('x'.repeat(190))
    const counter = document.querySelector<HTMLElement>('.counter')!
    expect(counter.textContent).toBe('190/190')
    expect(counter.classList.contains('over')).toBe(false)
    expect(button('Save').disabled).toBe(false)
    typeBio('x'.repeat(191))
    expect(counter.textContent).toBe('191/190')
    expect(counter.classList.contains('over')).toBe(true)
    expect(button('Save').disabled).toBe(true)
  })

  /** Settings opened on the Profile section, loaded. */
  async function openSettings(p: Profile = saved()) {
    handlers.get_profile = () => p
    app.state = { ...emptyState(), me: alex, people: { U1: { user: alex, online: true, last_seen_at: null } } } as never
    const onClose = vi.fn()
    cmp = mount(Settings, { target: document.body, props: { onClose, section: 'profile' } })
    await vi.waitFor(() => {
      flushSync()
      if (!document.querySelector('.preview .card')) throw new Error('not loaded')
    })
    return onClose
  }
  const closeButton = () => document.querySelector<HTMLButtonElement>('button[aria-label="Close settings"]')!
  const settle = () => vi.waitFor(() => { flushSync() })

  it('edits made while a Save is on its way are kept on top of the answer', async () => {
    let answer!: (p: Profile) => void
    handlers.update_profile = () => new Promise<Profile>((r) => (answer = r))
    const state = await open()
    click(option('Name colour', 'Custom'))
    setColor(document.querySelector<HTMLInputElement>('.colour input[type=color]')!, '#000000')
    click(button('Save'))
    click(option('Card theme', 'Berry')) // while saving
    typeBio('typed meanwhile')
    answer(saved({ user: { ...alex, style: { font: null, colors: ['#a8a8a8'] } } }))
    await vi.waitFor(() => {
      flushSync()
      expect(button('Save').disabled).toBe(false)
    })
    expect(option('Card theme', 'Berry').getAttribute('aria-checked')).toBe('true')
    expect(bioBox().textContent).toBe('typed meanwhile')
    // what wasn't touched since takes the server's answer (the lightened colour)
    expect(document.querySelector<HTMLInputElement>('.colour input[type=color]')!.value).toBe('#a8a8a8')
    expect(document.body.textContent).toContain('Unsaved changes')
    expect(state.dirty).toBe(true)
    expect(app.state.profiles.U1.user.style.colors).toEqual(['#a8a8a8'])
  })

  it('the answer reaches the cache even if the editor has gone', async () => {
    let answer!: (p: Profile) => void
    handlers.update_profile = () => new Promise<Profile>((r) => (answer = r))
    await open()
    click(option('Card theme', 'Berry'))
    click(button('Save'))
    unmount(cmp!)
    cmp = undefined
    const stored = saved({ card: { ...saved().card, theme_top: '#c2457a', theme_bottom: '#3a1f5e' } })
    answer(stored)
    await vi.waitFor(() => expect(app.state.profiles.U1).toEqual(stored))
  })

  it('closing during a Save waits for it: no discard prompt when it goes through', async () => {
    let answer!: (p: Profile) => void
    handlers.update_profile = () => new Promise<Profile>((r) => (answer = r))
    const onClose = await openSettings()
    click(option('Card theme', 'Berry'))
    click(button('Save'))
    click(closeButton())
    await settle()
    expect(document.body.textContent).not.toContain('Discard your profile changes?')
    expect(onClose).not.toHaveBeenCalled()
    const stored = saved({ card: { ...saved().card, theme_top: '#c2457a', theme_bottom: '#3a1f5e' } })
    answer(stored)
    await vi.waitFor(() => expect(onClose).toHaveBeenCalledTimes(1))
    expect(document.body.textContent).not.toContain('Discard your profile changes?')
    expect(app.state.profiles.U1).toEqual(stored)
  })

  it('a second way out during a Save wins over the first, and shows Saving… until it settles', async () => {
    let answer!: (p: Profile) => void
    handlers.update_profile = () => new Promise<Profile>((r) => (answer = r))
    const onClose = await openSettings()
    const section = (label: string) => [...document.querySelectorAll<HTMLButtonElement>('nav button')].find((b) => b.textContent === label)!
    click(option('Card theme', 'Berry'))
    click(button('Save'))
    click(closeButton())
    await settle()
    expect(document.querySelector('.content [role=status]')!.textContent).toBe('Saving your profile…')
    expect(closeButton().getAttribute('aria-busy')).toBe('true')
    click(section('Chat'))
    await settle()
    // the latest request is the one shown as waiting
    expect(closeButton().hasAttribute('aria-busy')).toBe(false)
    expect(section('Chat').getAttribute('aria-busy')).toBe('true')
    answer(saved({ card: { ...saved().card, theme_top: '#c2457a', theme_bottom: '#3a1f5e' } }))
    await vi.waitFor(() => {
      flushSync()
      expect(section('Chat').getAttribute('aria-current')).toBe('page')
    })
    expect(onClose).not.toHaveBeenCalled()
    expect(document.querySelector('.content [role=status]')).toBeNull()
    expect(section('Chat').hasAttribute('aria-busy')).toBe(false)
  })

  it('a Save started while the discard prompt shows puts the prompt away for good', async () => {
    let answer!: (p: Profile) => void
    handlers.update_profile = () => new Promise<Profile>((r) => (answer = r))
    const onClose = await openSettings()
    const section = (label: string) => [...document.querySelectorAll<HTMLButtonElement>('nav button')].find((b) => b.textContent === label)!
    click(option('Card theme', 'Berry'))
    click(closeButton())
    await settle()
    expect(document.body.textContent).toContain('Discard your profile changes?')
    click(button('Save'))
    await settle()
    expect(document.body.textContent).not.toContain('Discard your profile changes?')
    answer(saved({ card: { ...saved().card, theme_top: '#c2457a', theme_bottom: '#3a1f5e' } }))
    await vi.waitFor(() => {
      flushSync()
      expect(document.body.textContent).toContain('Saved')
    })
    expect(document.body.textContent).not.toContain('Discard your profile changes?')
    expect(onClose).not.toHaveBeenCalled()

    // and a way out asked for during the next Save isn't met by an older prompt either
    click(option('Card theme', 'Forest'))
    click(closeButton())
    await settle()
    expect(document.body.textContent).toContain('Discard your profile changes?')
    click(button('Save'))
    click(section('Chat'))
    await settle()
    expect(document.body.textContent).not.toContain('Discard your profile changes?')
    answer(saved())
    await vi.waitFor(() => {
      flushSync()
      expect(section('Chat').getAttribute('aria-current')).toBe('page')
    })
    expect(document.body.textContent).not.toContain('Discard your profile changes?')
    expect(onClose).not.toHaveBeenCalled()
  })

  it('asking for Profile again during a Save stays put', async () => {
    let answer!: (p: Profile) => void
    handlers.update_profile = () => new Promise<Profile>((r) => (answer = r))
    const onClose = await openSettings()
    const section = (label: string) => [...document.querySelectorAll<HTMLButtonElement>('nav button')].find((b) => b.textContent === label)!
    click(option('Card theme', 'Berry'))
    click(button('Save'))
    click(closeButton())
    click(section('Profile'))
    await settle()
    expect(document.querySelector('.content [role=status]')).toBeNull()
    answer(saved({ card: { ...saved().card, theme_top: '#c2457a', theme_bottom: '#3a1f5e' } }))
    await vi.waitFor(() => {
      flushSync()
      expect(document.body.textContent).toContain('Saved')
    })
    expect(onClose).not.toHaveBeenCalled()
    expect(section('Profile').getAttribute('aria-current')).toBe('page')
  })

  it('closing during a Save that fails asks, with the error shown', async () => {
    let refuse!: (e: unknown) => void
    handlers.update_profile = () => new Promise<Profile>((_, r) => (refuse = r))
    const onClose = await openSettings()
    click(option('Card theme', 'Berry'))
    click(button('Save'))
    click(closeButton())
    refuse('Colours must look like #a1b2c3.')
    await vi.waitFor(() => {
      flushSync()
      expect(document.body.textContent).toContain('Discard your profile changes?')
    })
    expect(document.querySelector('.footer [role=alert]')!.textContent).toBe('Colours must look like #a1b2c3.')
    expect(onClose).not.toHaveBeenCalled()
  })

  it('closing during an upload asks first', async () => {
    handlers.upload_avatar_media = () => new Promise<MediaInfo>(() => {})
    const onClose = await openSettings()
    await upload('Avatar')
    await vi.waitFor(() => {
      flushSync()
      expect(document.body.textContent).toContain('Uploading…')
    })
    click(closeButton())
    await settle()
    expect(document.body.textContent).toContain('Discard your profile changes?')
    expect(onClose).not.toHaveBeenCalled()
  })

  it('Escape while the discard prompt shows means Keep editing', async () => {
    const onClose = await openSettings()
    click(option('Card theme', 'Berry'))
    click(closeButton())
    await settle()
    expect(document.body.textContent).toContain('Discard your profile changes?')
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
    flushSync()
    expect(document.body.textContent).not.toContain('Discard your profile changes?')
    expect(onClose).not.toHaveBeenCalled()
    expect(option('Card theme', 'Berry').getAttribute('aria-checked')).toBe('true')
  })

  it('Reset during an upload drops it when it lands', async () => {
    let finish!: (m: MediaInfo) => void
    handlers.upload_avatar_media = () => new Promise<MediaInfo>((r) => (finish = r))
    const state = await open()
    await upload('Avatar')
    await vi.waitFor(() => {
      flushSync()
      expect(calls('upload_avatar_media')).toHaveLength(1)
    })
    expect(document.body.textContent).toContain('Uploading…')
    expect(state.dirty).toBe(true)
    click(button('Reset'))
    expect(document.body.textContent).not.toContain('Uploading…')
    finish(media('LATE'))
    await settle()
    await new Promise((r) => setTimeout(r, 0))
    flushSync()
    expect(previewLine().querySelector('img')).toBeNull()
    expect(state.dirty).toBe(false)
  })

  it('Remove during an upload cancels it', async () => {
    let finish!: (m: MediaInfo) => void
    handlers.upload_avatar_media = () => new Promise<MediaInfo>((r) => (finish = r))
    await open(saved({ user: { ...alex, avatar_hash: 'OLD' } }))
    await upload('Avatar')
    await vi.waitFor(() => {
      flushSync()
      expect(calls('upload_avatar_media')).toHaveLength(1)
    })
    click(button('Remove', document.querySelector('.avatar-field')!))
    finish(media('LATE'))
    await new Promise((r) => setTimeout(r, 0))
    flushSync()
    expect(previewLine().querySelector('img')).toBeNull()
    expect(document.body.textContent).not.toContain('Uploading…')
  })

  it('Reset of a staged upload restores the saved picture', async () => {
    handlers.upload_avatar_media = () => media('STAGED')
    const state = await open(saved({ user: { ...alex, avatar_hash: 'OLD' } }))
    await upload('Avatar')
    await vi.waitFor(() => {
      flushSync()
      expect(previewLine().querySelector('img')?.getAttribute('src')).toContain('STAGED')
    })
    click(button('Reset'))
    expect(previewLine().querySelector('img')!.getAttribute('src')).toContain('OLD')
    expect(state.dirty).toBe(false)
    expect(calls('update_profile')).toHaveLength(0)
  })

  it('more than 4 line breaks in the bio disables Save, blank leading lines not counted', async () => {
    await open()
    typeBio('\n\n\na\nb\nc\nd\ne')
    expect(document.body.textContent).not.toContain('Your bio can have at most 4 line breaks.')
    expect(button('Save').disabled).toBe(false)
    typeBio('a\nb\nc\nd\ne\nf')
    expect(document.querySelector('[role=alert]')!.textContent).toBe('Your bio can have at most 4 line breaks.')
    expect(button('Save').disabled).toBe(true)
  })

  it('pronouns count characters, not UTF-16 units, up to 40', async () => {
    await open()
    const box = document.querySelector<HTMLInputElement>('input[aria-label="Pronouns"]')!
    expect(box.maxLength).toBe(-1)
    type(box, '😀'.repeat(40))
    expect(document.body.textContent).not.toContain('Pronouns can be up to 40 characters.')
    expect(button('Save').disabled).toBe(false)
    type(box, '😀'.repeat(41))
    expect(document.querySelector('[role=alert]')!.textContent).toBe('Pronouns can be up to 40 characters.')
    expect(button('Save').disabled).toBe(true)
  })

  it('loads once you are known, and a failed load offers Retry', async () => {
    handlers.get_profile = () => Promise.reject('offline')
    app.state = emptyState()
    cmp = mount(ProfileSettings, { target: document.body, props: {} })
    flushSync()
    // waiting (#173): "Loading…" only once the load is slow (components/Loading.svelte), so nothing flashes
    expect(document.body.textContent).not.toContain('Loading…')
    expect(calls('get_profile')).toHaveLength(0)
    app.state = { ...emptyState(), me: alex, people: { U1: { user: alex, online: true, last_seen_at: null } } } as never
    await vi.waitFor(() => {
      flushSync()
      expect(document.querySelector('[role=alert]')?.textContent).toContain('offline')
    })
    handlers.get_profile = () => saved()
    click(button('Retry'))
    await vi.waitFor(() => {
      flushSync()
      if (!document.querySelector('.preview .card')) throw new Error('not loaded')
    })
    expect(calls('get_profile')).toHaveLength(2)
  })

  it('radio groups: one tab stop each, arrow keys move and pick', async () => {
    await open()
    showTab('Card')
    const radios = [...group('Card theme').querySelectorAll<HTMLButtonElement>('[role=radio]')]
    expect(radios.filter((r) => r.tabIndex === 0)).toEqual([option('Card theme', 'Plain')])
    const plain = radios.indexOf(option('Card theme', 'Plain'))
    radios[plain].focus()
    radios[plain].dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true, cancelable: true }))
    flushSync()
    const next = radios[(plain + 1) % radios.length]
    expect(document.activeElement).toBe(next)
    expect(next.getAttribute('aria-checked')).toBe('true')
    expect(next.tabIndex).toBe(0)
    expect(radios[plain].tabIndex).toBe(-1)
    // wraps backwards from the first
    showTab('Name style')
    const fonts = [...group('Name font').querySelectorAll<HTMLButtonElement>('[role=radio]')]
    fonts[0].focus()
    fonts[0].dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowLeft', bubbles: true, cancelable: true }))
    flushSync()
    expect(document.activeElement).toBe(fonts[fonts.length - 1])
    expect(fonts[fonts.length - 1].getAttribute('aria-checked')).toBe('true')
    const colours = [...group('Name colour').querySelectorAll<HTMLButtonElement>('[role=radio]')]
    expect(colours.filter((r) => r.tabIndex === 0)).toEqual([option('Name colour', 'Default')])
  })

  it('the footer sits flush with Settings\' padding through a shared variable', async () => {
    const { readFileSync } = await import('node:fs')
    const { resolve } = await import('node:path')
    const settings = readFileSync(resolve(__dirname, 'Settings.svelte'), 'utf8')
    const editor = readFileSync(resolve(__dirname, 'ProfileSettings.svelte'), 'utf8')
    expect(settings).toMatch(/\.content \{[^}]*--settings-pad-y: 28px;[^}]*padding: var\(--settings-pad-y\) 32px/)
    expect(editor).toMatch(/\.footer \{[^}]*bottom: calc\(-1 \* var\(--settings-pad-y, 28px\)\)/)
  })

  it('discard leaves the saved profile untouched', async () => {
    const people = { U1: { user: alex, online: true, last_seen_at: null } }
    handlers.get_profile = () => saved()
    handlers.upload_avatar_media = () => media('STAGED')
    app.state = { ...emptyState(), me: alex, people } as never
    const onClose = vi.fn()
    cmp = mount(Settings, { target: document.body, props: { onClose, section: 'profile' } })
    await vi.waitFor(() => {
      flushSync()
      if (!document.querySelector('.preview .card')) throw new Error('not loaded')
    })
    await upload('Avatar')
    await vi.waitFor(() => {
      flushSync()
      expect(previewLine().querySelector('img')?.getAttribute('src')).toContain('STAGED')
    })
    click(option('Name colour', 'Peach gradient'))
    click(document.querySelector<HTMLButtonElement>('button[aria-label="Close settings"]')!)
    expect(document.body.textContent).toContain('Discard your profile changes?')
    click(button('Discard'))
    expect(onClose).toHaveBeenCalledTimes(1)
    expect(calls('update_profile')).toHaveLength(0)
    expect(app.state.me).toBe(alex)
    expect(app.state.me!.avatar_hash).toBeNull()
    expect(app.state.people.U1.user).toBe(alex)
    expect(app.state.people.U1.user.style).toEqual({ font: null, colors: [] })
  })
})
