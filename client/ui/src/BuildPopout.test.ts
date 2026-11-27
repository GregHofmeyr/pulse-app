// @vitest-environment jsdom
// The build pop-out window's page (#620): the build the core hands it, read-only, never the app's store.
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { BuildSnapshot } from './lib/buildSnapshot'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args), convertFileSrc: (s: string) => s }))
// the pop-out must never load the app's store (or anything that does): importing it fails the test
vi.mock('./lib/store.svelte', () => {
  throw new Error('the pop-out loaded the store')
})

import BuildPopout from './BuildPopout.svelte'
import { settleTimers } from './lib/testSettle'

const snap = (s: Partial<BuildSnapshot> = {}): BuildSnapshot => ({
  message_id: 'M1', title: 'Bunny crit', game_name: 'The First Descendant', game_guessed: false, author_name: 'sam',
  saved_at: '2026-10-01T10:00:00.000Z', updated_at: null, notes: 'Max crit, **bunny** spam',
  link: { url: 'https://www.youtube.com/watch?v=dQw4w9WgXcQ', title: 'Bunny guide', thumb: null, provider: 'youtube', provider_id: 'dQw4w9WgXcQ' },
  screenshots: [{ id: 'A1', width: 1600, height: 900 }], archived: false, quick_drop: false, ...s,
})
let current: BuildSnapshot | null = snap()

let cmp: ReturnType<typeof mount> | undefined
const show = async () => {
  cmp = mount(BuildPopout, { target: document.body, props: {} })
  await settleTimers()
}
const calls = (cmd: string) => invoke.mock.calls.filter(([c]) => c === cmd).map(([, a]) => a)
const text = () => document.body.textContent ?? ''

beforeEach(() => {
  vi.useFakeTimers()
  current = snap()
  invoke.mockReset()
  invoke.mockImplementation(async (cmd: string, args?: unknown) => {
    if (cmd === 'build_window_snapshot') return current
    if (cmd === 'set_build_window_on_top') return true
    if (cmd === 'build_window_image_urls') {
      const id = (args as { messageId: string }).messageId
      return [{ id: 'A1', url: `https://x.r2.cloudflarestorage.com/${id}/A1?n=${calls('build_window_image_urls').length}`, expires_at: null }]
    }
    return undefined
  })
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  vi.useRealTimers()
})

describe('BuildPopout', () => {
  it('the pop-out renders the snapshot without the store', async () => {
    await show()
    expect(document.querySelector('.build-popout .bar .title')?.textContent).toBe('Bunny crit')
    expect(document.querySelector('.build-view .caption')?.textContent?.trim()).toBe('Builds · The First Descendant')
    expect(document.querySelector('.build-view h1')?.textContent).toBe('Bunny crit')
    expect(document.querySelector('.build-view .notes strong')?.textContent).toBe('bunny')
    expect(text()).not.toContain('Comments')
    // the stacked media rules, never the build page's filling row (#618)
    expect(document.querySelector('.build-view')?.getAttribute('data-layout')).toBe('stacked')
    expect(document.querySelector('.build-view .media.fill, .build-view .media.wide')).toBeNull()
    // only the pop-out's own three commands are ever called
    const used = new Set(invoke.mock.calls.map(([c]) => c))
    for (const c of used) expect(['build_window_snapshot', 'build_window_image_urls', 'set_build_window_on_top']).toContain(c)
  })

  it('asks for its screenshots by its own build, and again before the links lapse', async () => {
    await show()
    expect(calls('build_window_image_urls')).toEqual([{ messageId: 'M1' }])
    expect(document.querySelector<HTMLImageElement>('.build-view .tile img')?.src).toContain('/M1/A1?n=1')
    await vi.advanceTimersByTimeAsync(50 * 60_000)
    flushSync()
    expect(calls('build_window_image_urls').length).toBeGreaterThanOrEqual(2)
    expect(document.querySelector<HTMLImageElement>('.build-view .tile img')?.src).not.toContain('?n=1')
  })

  it('the video shows as a link card that opens in the browser', async () => {
    await show()
    const card = document.querySelector<HTMLAnchorElement>('a.link-card')!
    expect(card.href).toBe('https://www.youtube.com/watch?v=dQw4w9WgXcQ')
    expect(card.target).toBe('_blank')
    expect(card.textContent).toContain('opens in your browser')
    expect(document.querySelector('iframe')).toBeNull()
  })

  it('a new snapshot updates it', async () => {
    await show()
    current = snap({ title: 'Raven endgame', archived: true })
    await vi.advanceTimersByTimeAsync(2500)
    flushSync()
    expect(document.querySelector('.build-view h1')?.textContent).toBe('Raven endgame')
    expect(text()).toContain('ARCHIVED')
  })

  it('brought forward by another Pop out, it shows the new build at once, its screenshots by their own build (#639)', async () => {
    await show()
    current = snap({ message_id: 'M2', title: 'Bleed rogue' })
    window.dispatchEvent(new Event('focus'))
    await settleTimers()
    expect(document.querySelector('.build-popout .bar .title')?.textContent).toBe('Bleed rogue')
    expect(document.querySelector('.build-view h1')?.textContent).toBe('Bleed rogue')
    expect(calls('build_window_image_urls')).toEqual([{ messageId: 'M1' }, { messageId: 'M2' }])
  })

  it('the on-top switch says what it is, and each click calls set_build_window_on_top', async () => {
    await show()
    // asked once as it opens, re-stating the opening state: whether this window can be kept on top at all (QPMKJE)
    expect(calls('set_build_window_on_top')).toEqual([{ on: true }])
    const toggle = document.querySelector<HTMLButtonElement>('button.on-top')!
    // a labelled switch, not a bare pin icon whose clicks show nothing
    expect(toggle.textContent?.trim()).toBe('On top')
    expect(toggle.getAttribute('aria-pressed')).toBe('true')
    toggle.click()
    await settleTimers()
    expect(calls('set_build_window_on_top')).toEqual([{ on: true }, { on: false }])
    expect(toggle.getAttribute('aria-pressed')).toBe('false')
    toggle.click()
    await settleTimers()
    expect(calls('set_build_window_on_top')).toEqual([{ on: true }, { on: false }, { on: true }])
    expect(toggle.getAttribute('aria-pressed')).toBe('true')
  })

  it('where the window cannot be kept on top (Wayland) there is no switch (QPMKJE)', async () => {
    const base = invoke.getMockImplementation()!
    invoke.mockImplementation(async (cmd: string, args?: unknown) => (cmd === 'set_build_window_on_top' ? false : base(cmd, args)))
    await show()
    expect(calls('set_build_window_on_top')).toEqual([{ on: true }])
    expect(document.querySelector('button.on-top')).toBeNull()
    expect(document.querySelector('.build-popout .bar .title')?.textContent).toBe('Bunny crit')
  })

  it('a switch that fails says so and keeps its state (never a silent no-op)', async () => {
    await show()
    const base = invoke.getMockImplementation()!
    invoke.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === 'set_build_window_on_top') throw 'The pop-out is closed.'
      return base(cmd, args)
    })
    const toggle = document.querySelector<HTMLButtonElement>('button.on-top')!
    toggle.click()
    await settleTimers()
    expect(toggle.getAttribute('aria-pressed')).toBe('true')
    expect(document.querySelector('.build-popout [role=alert]')?.textContent).toContain("Couldn't change that")
  })

  it('with nothing to show it says the build is gone', async () => {
    current = null
    await show()
    expect(text()).toContain('This build is gone.')
    expect(document.querySelector('.build-view')).toBeNull()
  })
})

describe('main.ts', () => {
  const main = readFileSync(resolve(__dirname, 'main.ts'), 'utf8')
  it('mounts the pop-out page for ?window=build, and loads the app (and its store) only otherwise', () => {
    expect(main).not.toMatch(/^import App\b/m)
    expect(main).not.toMatch(/^import .*store\.svelte/m)
    expect(main).toMatch(/windowKind\(location\) === 'build'/)
    expect(main).toContain("import('./BuildPopout.svelte')")
    expect(main).toContain("import('./App.svelte')")
  })
})

// Greg's rule for the pop-out: nothing spills out of its container at any size down to the window's minimum (300 px
// wide, build_window.rs). jsdom can't lay out, so this checks the rules that keep it inside.
describe('BuildPopout at its narrowest', () => {
  const src = readFileSync(resolve(__dirname, 'BuildPopout.svelte'), 'utf8')
  const css = src.slice(src.indexOf('<style>'))
  it('the window minimum in the core is the width this page is laid out for', () => {
    const core = readFileSync(resolve(__dirname, '../../src-tauri/src/build_window.rs'), 'utf8')
    expect(core).toMatch(/\.min_inner_size\(300\.0, \d+\.0\)/)
  })
  it('sets no fixed width wider than the narrowest window', () => {
    for (const m of css.matchAll(/(?:^|[\s;{])(min-width|width|flex-basis)\s*:\s*(\d+)px/g)) {
      expect(Number(m[2]), m[0]).toBeLessThanOrEqual(260)
    }
  })
  it('wraps or ellipsises every text and keeps media inside', () => {
    for (const needle of [
      /\.bar \.title \{[^}]*text-overflow: ellipsis/,
      /\.bar \.title \{[^}]*min-width: 0/,
      /\.on-top \{[^}]*flex-shrink: 0/,
      /\.caption\)? ?[^{]*\{[^}]*overflow-wrap: anywhere/,
      /\.meta\)? ?[^{]*\{[^}]*overflow-wrap: anywhere/,
      /\.tiles\)? ?[^{]*\{[^}]*grid-template-columns: minmax\(0, 1fr\)/,
      /pre\)? ?[^{]*\{[^}]*overflow-x: auto/,
      /img\)? ?[^{]*\{[^}]*max-width: 100%/,
      /\.link-card \{[^}]*max-width: 100%/,
      /\.link-card \.host \{[^}]*text-overflow: ellipsis/,
      /\.scroll \{[^}]*overflow-x: hidden/,
    ]) {
      expect(css, String(needle)).toMatch(needle)
    }
  })
})
