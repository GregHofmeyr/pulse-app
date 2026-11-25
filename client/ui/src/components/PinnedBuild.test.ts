// @vitest-environment jsdom
// Pin to side (#619): the pinned build in the right column, kept current, and gone when deleted or out of reach.
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { flushSync, mount, unmount } from 'svelte'
import { compile } from 'svelte/compiler'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Build } from '../lib/protocol/Build'
import type { Event } from '../lib/protocol/Event'
import type { Message } from '../lib/protocol/Message'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args), convertFileSrc: (s: string) => s }))

import PinnedBuild from './PinnedBuild.svelte'
import { forgetPinnedBuild, notePinnedBuildEvent, pinBuild, pinnedBuild } from '../lib/pinnedBuild.svelte'
import { app, handleReady, resetState, ui } from '../lib/store.svelte'
import { applyEvent } from '../lib/state'
import { testMe, testReady } from '../lib/testReady'

const sam = { id: 'U2', username: 'sam', display_name: 'sam', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const ready = () =>
  testReady([{ id: 'B1', server_id: 'S1', kind: 'builds', name: 'raven-builds', position: 0 }], {
    members: [{ server_id: 'S1', members: [{ user: testMe, nickname: null }, { user: sam, nickname: null }] }],
    games: [{ id: 'G1', server_id: 'S1', name: 'The First Descendant', created_by: 'U1', created_at: '2026-10-01T00:00:00Z' }],
  })
const build = (id: string, b: Partial<Build> = {}): Message => ({
  id, channel_id: 'B1', author_id: 'U2', kind: 'normal', content: '', reply_to_id: null,
  created_at: '2026-10-01T10:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null,
  quote: null, qoty_card: null, embeds: [], clip: null,
  build: {
    title: `Build ${id}`, title_auto: false, game_id: 'G1', game_guessed: false, link: null, notes: 'Max crit, bunny spam', quick_drop: false,
    updated_at: null, archived_at: null, archived_by: null, reply_count: 0, last_reply_id: null, replies_seen: null, ...b,
  },
})

let cmp: ReturnType<typeof mount> | undefined
const settle = () => new Promise((r) => setTimeout(r, 0))
async function flush() {
  flushSync()
  await settle()
  flushSync()
}
const event = (e: Event) => {
  app.state = applyEvent(app.state, e, Date.now())
  notePinnedBuildEvent(e)
  flushSync()
}
const show = async () => {
  cmp = mount(PinnedBuild, { target: document.body, props: {} })
  await flush()
}
const text = () => document.querySelector('.pinned-build')?.textContent ?? ''

beforeEach(() => {
  try { localStorage.clear() } catch { /* none */ }
  invoke.mockReset()
  invoke.mockImplementation(async () => undefined)
  forgetPinnedBuild()
  resetState()
  handleReady(ready())
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('PinnedBuild', () => {
  it('shows the pinned build with Open and ×', async () => {
    pinBuild('B1', 'M001', build('M001'))
    await show()
    expect(document.querySelector('.pinned-build .bar .title')?.textContent).toBe('Build M001')
    expect(document.querySelector('.pinned-build .build-view .caption')?.textContent?.trim()).toBe('Builds · The First Descendant')
    expect(text()).toContain('Max crit, bunny spam')
    document.querySelector<HTMLButtonElement>('.pinned-build button.open-page')!.click()
    expect(ui.goTo).toBe('B1')
    expect(ui.openBuild).toEqual({ channelId: 'B1', messageId: 'M001' })
    document.querySelector<HTMLButtonElement>('.pinned-build button[aria-label="Unpin"]')!.click()
    expect(pinnedBuild.key).toBeNull()
    expect(localStorage.getItem('pulse.pinnedBuild.U1')).toBeNull()
  })

  it('an edit and an archive show at once', async () => {
    pinBuild('B1', 'M001', build('M001'))
    await show()
    event({ t: 'MessageUpdated', d: { message: build('M001', { title: 'Raven endgame' }) } })
    expect(document.querySelector('.pinned-build h1')?.textContent).toBe('Raven endgame')
    event({ t: 'BuildsArchived', d: { channel_id: 'B1', message_ids: ['M001'], archived_at: '2026-10-05T00:00:00Z', archived_by: 'U2' } })
    expect(text()).toContain('ARCHIVED')
  })

  // Review Focus 5
  it('a_pinned_build_that_is_deleted_unpins_with_a_note', async () => {
    pinBuild('B1', 'M001', build('M001'))
    expect(localStorage.getItem('pulse.pinnedBuild.U1')).not.toBeNull()
    await show()
    event({ t: 'MessageDeleted', d: { channel_id: 'B1', message_id: 'M001', author_id: 'U2', mentions: [], thread_root_id: null } } as Event)
    expect(text()).toContain('This build is gone.')
    expect(document.querySelector('.pinned-build .build-view')).toBeNull()
    expect(localStorage.getItem('pulse.pinnedBuild.U1'), 'a restart does not bring it back').toBeNull()
    document.querySelector<HTMLButtonElement>('.pinned-build button.dismiss')!.click()
    expect(pinnedBuild.key).toBeNull()
  })

  it('losing access (its channel leaves the data) says it is gone', async () => {
    pinBuild('B1', 'M001', build('M001'))
    await show()
    event({ t: 'ChannelDeleted', d: { channel_id: 'B1', server_id: 'S1' } })
    expect(text()).toContain('This build is gone.')
  })

  it('a refetch that finds nothing says it is gone; one that fails offline keeps the copy', async () => {
    invoke.mockImplementation(async (cmd: string) => { if (cmd === 'get_build') throw "can't reach the server: offline" })
    pinBuild('B1', 'M001', build('M001'))
    handleReady(ready())
    await flush()
    await show()
    expect(text()).toContain('Build M001')
    invoke.mockImplementation(async (cmd: string) => { if (cmd === 'get_build') throw 'not found' })
    handleReady(ready())
    await flush()
    expect(text()).toContain('This build is gone.')
  })

  // #636 review: it is a right column like the members list, its width the resize handle's
  it('takes the right column width the handle sets', () => {
    const src = readFileSync(resolve(__dirname, 'PinnedBuild.svelte'), 'utf8')
    const css = compile(src, { filename: 'PinnedBuild.svelte', css: 'external', cssHash: () => 'x' }).css!.code
    const rule = css.match(/\.pinned-build\.x\s*\{([^}]*)\}/)?.[1] ?? ''
    expect(rule).toMatch(/width:\s*var\(--right-w,\s*320px\)/)
    expect(rule).toMatch(/flex-shrink:\s*0/)
    expect(rule).toMatch(/background:\s*var\(--bg-1\)/)
  })

  // #638: nothing spills out of the column at its least width (jsdom doesn't lay out: the rules that keep it in)
  it('its contents stay inside the column at any width', () => {
    const cssOf = (file: string) => compile(readFileSync(resolve(__dirname, file), 'utf8'), { filename: file, css: 'external', cssHash: () => 'x' }).css!.code
    const rule = (css: string, sel: string) => css.match(new RegExp(`${sel}\\s*\\{([^}]*)\\}`))?.[1] ?? ''
    const pinned = cssOf('PinnedBuild.svelte')
    expect(rule(pinned, '\\.title\\.x')).toMatch(/min-width:\s*0[\s\S]*text-overflow:\s*ellipsis/)
    expect(rule(pinned, '\\.bar\\.x \\.btn:where\\(\\.x\\), \\.bar\\.x \\.icon-btn:where\\(\\.x\\)')).toMatch(/flex-shrink:\s*0/)
    const view = cssOf('BuildView.svelte')
    expect(rule(view, '\\.caption\\.x')).toMatch(/overflow-wrap:\s*anywhere/)
    expect(rule(view, '\\.meta\\.x')).toMatch(/overflow-wrap:\s*anywhere/)
    expect(rule(view, '\\.badges\\.x')).toMatch(/flex-wrap:\s*wrap/)
    expect(rule(view, '\\.tiles\\.x')).toMatch(/minmax\(140px, 1fr\)|minmax\(min\(140px, 100%\), 1fr\)/)
    expect(view).toMatch(/\.notes\.x pre[^{]*\{[^}]*white-space:\s*pre-wrap/)
    expect(view).toMatch(/\.notes\.x img[^{]*\{[^}]*max-width:\s*100%/)
  })
})
