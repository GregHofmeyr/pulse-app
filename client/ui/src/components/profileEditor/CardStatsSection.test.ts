// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fakeMatchMedia, FakeIntersectionObserver } from '../../lib/testMotion'

vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))

import CardStatsSection from './CardStatsSection.svelte'
import CardEditor from './CardEditor.svelte'
import { ProfileEditorState } from '../../lib/profileEditor.svelte'
import { app } from '../../lib/store.svelte'
import { emptyState } from '../../lib/state'
import type { Profile } from '../../lib/protocol/Profile'
import type { User } from '../../lib/protocol/User'

const alex: User = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const profile = (shown?: string[]): Profile => ({
  user: alex,
  card: { banner_hash: null, banner_still: null, theme_top: '#3a3d46', theme_bottom: '#16171b', pronouns: null, bio: null },
  stats: { voice_seconds: 9000, member_since: '2026-10-03T00:00:00Z', top_reaction: null, times_renamed: 0, steam_completion: 50, steam_perfect_games: 1, steam_recent_minutes: 600, ...(shown ? { shown } : {}) },
  badges: [],
  name_archive: [],
  showcase: [],
  showcase_chosen: false,
})

let cmp: ReturnType<typeof mount> | undefined
let st: ProfileEditorState
beforeEach(() => {
  app.state = { ...emptyState(), me: alex } as never
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

function show(p = profile(), component: typeof CardStatsSection | typeof CardEditor = CardStatsSection) {
  st = new ProfileEditorState(p)
  cmp = mount(component as never, { target: document.body, props: { state: st } })
  flushSync()
}
const q = <T extends Element = HTMLElement>(sel: string) => document.querySelector<T>(sel)!
const qa = (sel: string) => [...document.querySelectorAll<HTMLElement>(sel)]
const click = (el: Element) => { (el as HTMLElement).click(); flushSync() }
const rowKeys = () => qa('[data-row]').map((r) => r.dataset.row)
const sw = (key: string) => q(`[data-row="${key}"] [role="switch"]`)
const key = (el: Element, k: string) => { el.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true })); flushSync() }

function pointer(type: string, el: Element | Window, x: number, y = 0) {
  const e = new MouseEvent(type, { bubbles: true, cancelable: true, clientX: x, clientY: y, button: 0 })
  Object.defineProperty(e, 'pointerId', { value: 1 })
  el.dispatchEvent(e)
}
const rect = (left: number, top: number, w: number, h: number) => ({ left, top, right: left + w, bottom: top + h, width: w, height: h, x: left, y: top, toJSON: () => ({}) })
/** Rows 40px tall, 8px apart, one under another. */
const layout = () => qa('[data-stat]').forEach((r, i) => (r.getBoundingClientRect = () => rect(0, i * 48, 300, 40)))

describe('the Card stats section (#727)', () => {
  it('rows_with_switch_and_drag_handle', () => {
    show()
    expect(rowKeys()).toEqual(['member_since', 'voice', 'steam_completion', 'steam_recent'])
    for (const r of qa('[data-row]')) {
      expect(r.querySelector('[role="switch"]')).not.toBeNull()
      expect(r.querySelector('.handle')).not.toBeNull()
    }
    expect(qa('[role="switch"]').every((s) => s.getAttribute('aria-checked') === 'true')).toBe(true)
    // a saved choice: the shown ones first, in order, on; the rest after, off
    unmount(cmp!)
    show(profile(['voice', 'steam_recent']))
    expect(rowKeys()).toEqual(['voice', 'steam_recent', 'member_since', 'steam_completion'])
    expect(qa('[role="switch"]').map((s) => s.getAttribute('aria-checked'))).toEqual(['true', 'true', 'false', 'false'])
  })

  it('toggle_and_reorder_update_card_live', async () => {
    show(profile(), CardEditor)
    const card = () => [...document.querySelectorAll('.info dt')].map((d) => d.textContent)
    expect(card()).toEqual(['Member since', 'Voice', 'Steam', 'Steam, 2 weeks'])
    unmount(cmp!)
    show()
    click(sw('voice'))
    expect(st.draft.cardStats).toEqual(['member_since', 'steam_completion', 'steam_recent'])
    expect(sw('voice').getAttribute('aria-checked')).toBe('false')
    expect(rowKeys()).toEqual(['member_since', 'steam_completion', 'steam_recent', 'voice'])
    // switched back on, it joins the end of the shown ones
    click(sw('voice'))
    expect(st.draft.cardStats).toEqual(['member_since', 'steam_completion', 'steam_recent', 'voice'])
    // keyboard: arrows on the handle move a row
    key(q('[data-stat="voice"] .handle'), 'ArrowUp')
    expect(st.draft.cardStats).toEqual(['member_since', 'steam_completion', 'voice', 'steam_recent'])
    expect(document.activeElement).toBe(q('[data-stat="voice"] .handle'))
    key(q('[data-stat="member_since"] .handle'), 'ArrowUp')
    expect(st.draft.cardStats![0]).toBe('member_since')
    // drag: the first row down past the second
    layout()
    pointer('pointerdown', q('[data-stat="member_since"]'), 20, 20)
    pointer('pointermove', q('.stats'), 20, 70)
    pointer('pointerup', q('.stats'), 20, 70)
    flushSync()
    expect(st.draft.cardStats).toEqual(['steam_completion', 'member_since', 'voice', 'steam_recent'])
    expect(st.dirty).toBe(true)
    // the click a drag ends with is swallowed by the drag; a later one is a fresh click
    await new Promise((r) => setTimeout(r, 0))
    // all off is allowed
    for (const k of ['steam_completion', 'member_since', 'voice', 'steam_recent']) click(sw(k))
    expect(st.draft.cardStats).toEqual([])
  })

  it('card_follows_the_draft', () => {
    show()
    unmount(cmp!)
    show(profile(), CardEditor)
    st.draft.cardStats = ['steam_recent', 'member_since']
    flushSync()
    expect([...document.querySelectorAll('.info dt')].map((d) => d.textContent)).toEqual(['Steam, 2 weeks', 'Member since'])
  })

  it('steam_rows_say_they_show_only_while_sharing', () => {
    show()
    for (const k of ['steam_completion', 'steam_recent']) {
      expect(q(`[data-row="${k}"] .name`).dataset.tip).toMatch(/share Steam/)
      expect(sw(k).dataset.tip).toMatch(/share Steam/)
    }
    expect(q('[data-row="voice"] .name').dataset.tip).toBeUndefined()
  })

  it('a_press_that_moves_without_dragging_is_not_a_toggle', () => {
    show()
    layout()
    // sideways past the threshold: no drag starts on a vertical list, and the click that follows is not a toggle
    pointer('pointerdown', sw('voice'), 20, 60)
    pointer('pointermove', q('.stats'), 40, 60)
    pointer('pointerup', q('.stats'), 40, 60)
    click(sw('voice'))
    expect(st.draft.cardStats).toBeNull()
    // an ordinary click still toggles
    pointer('pointerdown', sw('voice'), 20, 60)
    pointer('pointerup', sw('voice'), 20, 60)
    click(sw('voice'))
    expect(st.draft.cardStats).toEqual(['member_since', 'steam_completion', 'steam_recent'])
  })
})
