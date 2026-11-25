// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Build } from '../lib/protocol/Build'
import type { Message } from '../lib/protocol/Message'
import { FakeIntersectionObserver, fakeMatchMedia } from '../lib/testMotion'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args), convertFileSrc: (s: string) => s }))
vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)

import BuildQuickAdd from './BuildQuickAdd.svelte'
import BuildsChannel from './BuildsChannel.svelte'
import { app, handleReady, resetState } from '../lib/store.svelte'
import { applyEvent } from '../lib/state'
import { testMe, testReady } from '../lib/testReady'

const sam = { id: 'U2', username: 'sam', display_name: 'sam', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const ready = () =>
  testReady([{ id: 'B1', server_id: 'S1', kind: 'builds', name: 'raven-builds', position: 0 }], {
    members: [{ server_id: 'S1', members: [{ user: testMe, nickname: null }, { user: sam, nickname: null }] }],
    read_states: [{ channel_id: 'B1', last_read_message_id: null, unread: 0, mentions: 0 }],
    games: [{ id: 'G1', server_id: 'S1', name: 'The First Descendant', created_by: 'U1', created_at: '2026-10-01T00:00:00Z' }],
  })

const LINK = 'https://www.youtube.com/watch?v=dQw4w9WgXcQ'
const build = (id: string, b: Partial<Build> = {}, m: Partial<Message> = {}): Message => ({
  id, channel_id: 'B1', author_id: 'U1', kind: 'normal', content: '', reply_to_id: null,
  created_at: '2026-10-01T10:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null,
  quote: null, qoty_card: null, embeds: [], clip: null,
  build: {
    title: 'youtube.com/watch?v=dQw4w9WgXcQ', title_auto: true, game_id: null, game_guessed: false,
    link: { url: LINK, title: null, thumb: null, provider: 'generic', provider_id: null }, notes: null, quick_drop: true,
    updated_at: null, archived_at: null, archived_by: null, reply_count: 0, last_reply_id: null, replies_seen: null, ...b,
  },
  ...m,
})
const staged = { id: 'st1', name: 'image.webp', size: 500, content_type: 'image/webp', kind: 'image', width: 40, height: 20, preview: null }

let cmp: ReturnType<typeof mount> | undefined
const settle = () => new Promise((r) => setTimeout(r, 0))
const calls = (cmd: string) => invoke.mock.calls.filter((c) => c[0] === cmd)
let created: Message = build('M100')
/** Holds create_build's answer until released (the build page's "pending" moment). */
let release: () => void = () => {}

async function flush() {
  for (let i = 0; i < 3; i++) {
    flushSync()
    await settle()
  }
  flushSync()
}

beforeEach(() => {
  Element.prototype.scrollTo = () => {}
  try { localStorage.clear() } catch { /* none */ }
  invoke.mockReset()
  created = build('M100')
  invoke.mockImplementation(async (cmd: string) => {
    if (cmd === 'list_builds') return []
    if (cmd === 'build_facets') return { total: 0, games: [] }
    if (cmd === 'stage_attachment_bytes') return staged
    if (cmd === 'create_build') return new Promise((r) => (release = () => r(created)))
    if (cmd === 'update_build') return created
    return undefined
  })
  resetState()
  handleReady(ready())
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

async function channel() {
  cmp = mount(BuildsChannel, { target: document.body, props: { channelId: 'B1', serverId: 'S1' } })
  await flush()
}
function paste(target: Element, data: { files?: File[]; text?: string }) {
  const e = new Event('paste', { bubbles: true, cancelable: true }) as ClipboardEvent
  Object.defineProperty(e, 'clipboardData', { value: { files: data.files ?? [], getData: (t: string) => (t === 'text/plain' ? (data.text ?? '') : '') } })
  target.dispatchEvent(e)
  return e
}
const field = () => document.querySelector<HTMLInputElement>('.quick-add input')!
function type(text: string) {
  const f = field()
  f.value = text
  f.dispatchEvent(new Event('input', { bubbles: true }))
  flushSync()
}
function enter() {
  field().dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }))
  flushSync()
}
const addButton = () => [...document.querySelectorAll<HTMLButtonElement>('.builds-head button')].find((b) => b.textContent?.trim() === '+ Add build')!
const body = () => (calls('create_build')[0][1] as { body: Record<string, unknown> }).body

describe('BuildQuickAdd', () => {
  it('pasting a link and Enter saves a quick drop', async () => {
    await channel()
    addButton().click()
    await flush()
    expect(document.activeElement).toBe(field())
    // one live region, there from the start, that says what happens
    const live = document.querySelectorAll('.quick-add [aria-live]')
    expect(live).toHaveLength(1)
    paste(field(), { text: LINK })
    // the field takes the pasted text as typed (the browser's own paste); here, as typed
    type(LINK)
    enter()
    await flush()
    expect(calls('create_build')).toHaveLength(1)
    const args = calls('create_build')[0][1] as { channelId: string; staged: string[] }
    expect(args.channelId).toBe('B1')
    expect(args.staged).toEqual([])
    expect(body()).toMatchObject({ title: null, game_id: null, link_url: LINK, notes: null, attachments: [] })
    expect(typeof body().nonce).toBe('string')
    // shown as saving until the build arrives
    expect(document.querySelector('.quick-add .saved.pending')).not.toBeNull()
    expect(document.querySelectorAll('.quick-add [aria-live]')).toHaveLength(1)
    expect(document.querySelector('.quick-add [aria-live]')).toBe(live[0])
    release()
    await flush()
    app.state = applyEvent(app.state, { t: 'MessageCreated', d: { message: created, nonce: null } }, Date.now())
    await flush()
    expect(document.querySelector('.quick-add .saved.pending')).toBeNull()
    expect(document.querySelector('.quick-add .saved .title')?.textContent).toBe('youtube.com/watch?v=dQw4w9WgXcQ')
    expect(field().value).toBe('')
  })

  it('the title fills in when the unfurl lands', async () => {
    await channel()
    addButton().click()
    await flush()
    type(LINK)
    enter()
    release()
    await flush()
    app.state = applyEvent(app.state, { t: 'MessageCreated', d: { message: created, nonce: null } }, Date.now())
    await flush()
    const unfurled = build('M100', { title: 'Raven bunny build', link: { url: LINK, title: 'Raven bunny build', thumb: null, provider: 'youtube', provider_id: 'dQw4w9WgXcQ' } })
    app.state = applyEvent(app.state, { t: 'MessageUpdated', d: { message: unfurled } }, Date.now())
    await flush()
    expect(document.querySelector('.quick-add .saved .title')?.textContent).toBe('Raven bunny build')
    // and its card in the gallery too
    expect(document.querySelector('[data-build-id="M100"] .title')?.textContent).toBe('Raven bunny build')
  })

  it('a guessed game shows guessed and one click clears it', async () => {
    await channel()
    addButton().click()
    await flush()
    type(LINK)
    enter()
    release()
    await flush()
    const guessed = build('M100', { title: 'TFD Raven bunny build', game_id: 'G1', game_guessed: true })
    app.state = applyEvent(app.state, { t: 'MessageCreated', d: { message: guessed, nonce: null } }, Date.now())
    await flush()
    expect(document.querySelector('.quick-add .saved .caption')?.textContent).toContain('The First Descendant')
    expect(document.querySelector('.quick-add .saved .auto-picked')?.textContent).toBe(' · auto picked')
    document.querySelector<HTMLButtonElement>('.quick-add .saved .clear-game')!.click()
    await flush()
    expect(calls('update_build')[0][1]).toEqual({
      messageId: 'M100', channelId: 'B1',
      body: { title: 'TFD Raven bunny build', game_id: null, link_url: LINK, notes: null, attachments: [], mark_sorted: false },
      order: [],
    })
  })

  it('pasting an image saves an Untitled build', async () => {
    await channel()
    addButton().click()
    await flush()
    const png = new File([new Uint8Array([137, 80, 78, 71])], 'image.png', { type: 'image/png' })
    const e = paste(field(), { files: [png] })
    expect(e.defaultPrevented).toBe(true)
    await flush()
    expect(calls('stage_attachment_bytes')).toHaveLength(1)
    expect(document.querySelector('.quick-add .shot')?.textContent).toContain('image.webp')
    enter()
    await flush()
    expect((calls('create_build')[0][1] as { staged: string[] }).staged).toEqual(['st1'])
    expect(body()).toMatchObject({ title: null, link_url: null })
  })

  it('typed text becomes the title', async () => {
    await channel()
    addButton().click()
    await flush()
    type('  Raven crit build ')
    enter()
    await flush()
    expect(body()).toMatchObject({ title: 'Raven crit build', link_url: null, game_id: null, notes: null })
  })

  it('Add details opens the form instead', async () => {
    const onDetails = vi.fn()
    cmp = mount(BuildQuickAdd, { target: document.body, props: { channelId: 'B1', serverId: 'S1', onDetails, onClose: () => {} } })
    await flush()
    type(LINK)
    document.querySelector<HTMLButtonElement>('.quick-add .details')!.click()
    await flush()
    expect(onDetails).toHaveBeenCalledWith({ title: '', link: LINK, files: [] })
    expect(calls('create_build')).toHaveLength(0)
    unmount(cmp!)
    // in the channel: the full form, prefilled, in place of the quick field
    await channel()
    addButton().click()
    await flush()
    type(LINK)
    document.querySelector<HTMLButtonElement>('.quick-add .details')!.click()
    await flush()
    expect(document.querySelector('.quick-add')).toBeNull()
    expect(document.querySelector<HTMLInputElement>('.build-form input[aria-label="Build link"]')?.value).toBe(LINK)
    expect(calls('create_build')).toHaveLength(0)
  })

  it('a failed save keeps its nonce for the retry, and closing gives it up', async () => {
    let fail = true
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_builds') return []
      if (cmd === 'build_facets') return { total: 0, games: [] }
      if (cmd === 'stage_attachment_bytes') return staged
      if (cmd === 'create_build') {
        if (fail) throw 'can\'t reach the server'
        return created
      }
      return undefined
    })
    await channel()
    addButton().click()
    await flush()
    paste(field(), { files: [new File([new Uint8Array([137])], 'image.png', { type: 'image/png' })] })
    await flush()
    type('Raven crit')
    enter()
    await flush()
    expect(document.querySelector('.quick-add .error')).not.toBeNull()
    // put back as it was: the retry is the same save
    expect(field().value).toBe('Raven crit')
    enter()
    await flush()
    const nonces = () => calls('create_build').map((c) => (c[1] as { body: { nonce: string } }).body.nonce)
    expect(nonces()).toHaveLength(2)
    expect(nonces()[1]).toBe(nonces()[0])
    // changed: a new save, and the old one's uploads are let go
    type('Raven crit v2')
    enter()
    await flush()
    expect(nonces()[2]).not.toBe(nonces()[0])
    expect(calls('cancel_build_save').map((c) => c[1])).toEqual([{ nonce: nonces()[0] }])
    // closing after it failed gives up that save too, and its screenshot
    document.querySelector<HTMLButtonElement>('.quick-add button[aria-label="Close quick add"]')!.click()
    await flush()
    expect(calls('cancel_build_save').map((c) => c[1])).toEqual([{ nonce: nonces()[0] }, { nonce: nonces()[2] }])
    expect(calls('unstage_attachment').map((c) => c[1])).toEqual([{ id: 'st1' }])
    expect(document.querySelector('.quick-add')).toBeNull()
  })

  it('a paste in the search box stays in the search box', async () => {
    await channel()
    const search = document.querySelector<HTMLInputElement>('input[type=search]')!
    const e = paste(search, { text: LINK })
    await flush()
    expect(e.defaultPrevented).toBe(false)
    expect(document.querySelector('.quick-add')).toBeNull()
    // anywhere else in the browse view, it goes to quick add
    paste(document.querySelector('.feed')!, { text: LINK })
    await flush()
    expect(field().value).toBe(LINK)
  })
})
