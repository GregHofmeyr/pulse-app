// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Attachment } from '../lib/protocol/Attachment'
import type { Build } from '../lib/protocol/Build'
import type { Message } from '../lib/protocol/Message'
import { FakeIntersectionObserver, fakeMatchMedia } from '../lib/testMotion'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args), convertFileSrc: (s: string) => s }))
vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)

import BuildForm from './BuildForm.svelte'
import BuildPage from './BuildPage.svelte'
import { handleReady, resetState } from '../lib/store.svelte'
import { testMe, testReady } from '../lib/testReady'

const ready = () =>
  testReady([{ id: 'B1', server_id: 'S1', kind: 'builds', name: 'raven-builds', position: 0 }], {
    members: [{ server_id: 'S1', members: [{ user: testMe, nickname: null }] }],
    games: [{ id: 'G1', server_id: 'S1', name: 'The First Descendant', created_by: 'U1', created_at: '2026-10-01T00:00:00Z' }],
  })

const shotAtt = (n: number): Attachment => ({
  id: `A${n}`, name: `shot${n}.webp`, content_type: 'image/webp', kind: 'image', size: 100, width: 1600, height: 900, duration_ms: null, expires_at: null, expired: false,
})
const build = (b: Partial<Build> = {}, m: Partial<Message> = {}): Message => ({
  id: 'M001', channel_id: 'B1', author_id: 'U1', kind: 'normal', content: '', reply_to_id: null,
  created_at: '2026-10-01T10:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null,
  quote: null, qoty_card: null, embeds: [], clip: null,
  build: {
    title: 'Raven bunny', title_auto: false, game_id: 'G1', game_guessed: false,
    link: { url: 'https://www.youtube.com/watch?v=dQw4w9WgXcQ', title: null, thumb: null, provider: 'youtube', provider_id: 'dQw4w9WgXcQ' },
    notes: 'Max crit', quick_drop: false, updated_at: null, archived_at: null, archived_by: null, reply_count: 0, last_reply_id: null, replies_seen: null, ...b,
  },
  ...m,
})

let n = 0
const staged = (name: string) => ({ id: `st${++n}`, name, size: 500, content_type: 'image/webp', kind: 'image', width: 40, height: 20, preview: null })
let cmp: ReturnType<typeof mount> | undefined
const settle = () => new Promise((r) => setTimeout(r, 0))
const calls = (cmd: string) => invoke.mock.calls.filter((c) => c[0] === cmd)
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
  n = 0
  invoke.mockReset()
  invoke.mockImplementation(async (cmd: string, args?: unknown) => {
    if (cmd === 'stage_attachment_bytes') return staged('pasted.webp')
    if (cmd === 'attachment_urls') return []
    if (cmd === 'list_replies') return []
    if (cmd === 'add_game') return { id: 'G2', server_id: 'S1', name: (args as { name: string }).name, created_by: 'U1', created_at: '2026-10-01T00:00:00Z' }
    if (cmd === 'create_build' || cmd === 'update_build') return build()
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

const onClose = vi.fn()
async function form(props: Record<string, unknown> = {}) {
  cmp = mount(BuildForm, { target: document.body, props: { channelId: 'B1', serverId: 'S1', onClose, ...props } })
  await flush()
}
const q = <T extends Element = HTMLElement>(s: string) => document.querySelector<T>(`.build-form ${s}`)!
function typeIn(el: HTMLInputElement | HTMLTextAreaElement, text: string) {
  el.value = text
  el.dispatchEvent(new Event('input', { bubbles: true }))
  flushSync()
}
function paste(target: Element, data: { files?: File[]; text?: string }) {
  const e = new Event('paste', { bubbles: true, cancelable: true }) as ClipboardEvent
  Object.defineProperty(e, 'clipboardData', { value: { files: data.files ?? [], getData: (t: string) => (t === 'text/plain' ? (data.text ?? '') : '') } })
  target.dispatchEvent(e)
  return e
}
const png = (name = 'image.png') => new File([new Uint8Array([137, 80, 78, 71])], name, { type: 'image/png' })
const shotNames = () => [...document.querySelectorAll('.build-form .shots li .name')].map((e) => e.textContent?.trim())
const save = () => q<HTMLButtonElement>('button.save')

describe('BuildForm', () => {
  it('pasting an image while typing notes adds a screenshot and keeps the notes', async () => {
    await form()
    const notes = q<HTMLTextAreaElement>('textarea[aria-label="Notes"]')
    notes.focus()
    typeIn(notes, 'Max crit, bunny spam')
    const e = paste(notes, { files: [png()] })
    expect(e.defaultPrevented).toBe(true)
    await flush()
    expect(shotNames()).toEqual(['pasted.webp'])
    expect(notes.value).toBe('Max crit, bunny spam')
  })

  it('pasting text in notes stays text', async () => {
    await form()
    const notes = q<HTMLTextAreaElement>('textarea[aria-label="Notes"]')
    const e = paste(notes, { text: 'Use the Raven module' })
    expect(e.defaultPrevented).toBe(false)
    await flush()
    expect(calls('stage_attachment_bytes')).toHaveLength(0)
    expect(shotNames()).toEqual([])
  })

  it('screenshots reorder and remove', async () => {
    await form({ build: build({}, { attachments: [1, 2, 3].map(shotAtt) }) })
    expect(shotNames()).toEqual(['shot1.webp', 'shot2.webp', 'shot3.webp'])
    document.querySelectorAll<HTMLButtonElement>('.build-form .shots li')[2].querySelector<HTMLButtonElement>('button[aria-label^="Move earlier"]')!.click()
    flushSync()
    expect(shotNames()).toEqual(['shot1.webp', 'shot3.webp', 'shot2.webp'])
    document.querySelectorAll<HTMLButtonElement>('.build-form .shots li')[0].querySelector<HTMLButtonElement>('button[aria-label^="Remove"]')!.click()
    flushSync()
    expect(shotNames()).toEqual(['shot3.webp', 'shot2.webp'])
    // a new one goes in its place in the order
    paste(q('textarea[aria-label="Notes"]'), { files: [png()] })
    await flush()
    save().click()
    await flush()
    expect((calls('update_build')[0][1] as { order: string[] }).order).toEqual(['id:A3', 'id:A2', 'staged:st1'])
  })

  it('an 11th screenshot is refused with a note', async () => {
    await form({ build: build({}, { attachments: Array.from({ length: 10 }, (_, i) => shotAtt(i)) }) })
    expect(shotNames()).toHaveLength(10)
    paste(q('textarea[aria-label="Notes"]'), { files: [png()] })
    await flush()
    expect(shotNames()).toHaveLength(10)
    expect(calls('stage_attachment_bytes')).toHaveLength(0)
    expect(q('.shots-note').textContent).toBe('Up to 10 screenshots.')
  })

  it('a new game name is added to the registry', async () => {
    await form()
    const game = q<HTMLInputElement>('input[aria-label="Game"]')
    game.focus()
    typeIn(game, 'Warframe')
    const add = [...document.querySelectorAll<HTMLElement>('.build-form [role=option]')].find((o) => o.textContent?.includes('Warframe'))!
    expect(add.textContent?.trim()).toBe("+ Use 'Warframe' as a new game")
    add.click()
    await flush()
    expect(calls('add_game')[0][1]).toEqual({ serverId: 'S1', name: 'Warframe' })
    typeIn(q<HTMLInputElement>('input[aria-label="Title"]'), 'Excalibur')
    save().click()
    await flush()
    expect((calls('create_build')[0][1] as { body: { game_id: string } }).body.game_id).toBe('G2')
  })

  it('Save is disabled without a title', async () => {
    await form({ prefill: { title: '', link: 'https://planner.example/raven', files: [] } })
    expect(q<HTMLInputElement>('input[aria-label="Build link"]').value).toBe('https://planner.example/raven')
    expect(save().disabled).toBe(true)
    const title = q<HTMLInputElement>('input[aria-label="Title"]')
    typeIn(title, 'Raven planner')
    expect(save().disabled).toBe(false)
    expect(q('.counter').textContent).toBe('13/120')
    save().click()
    await flush()
    const args = calls('create_build')[0][1] as { channelId: string; body: Record<string, unknown>; staged: string[] }
    expect(args.channelId).toBe('B1')
    expect(args.body).toMatchObject({ title: 'Raven planner', game_id: null, link_url: 'https://planner.example/raven', notes: null, attachments: [] })
    expect(args.staged).toEqual([])
    expect(onClose).toHaveBeenCalled()
  })

  it('Edit opens prefilled and saves an update', async () => {
    cmp = mount(BuildPage, { target: document.body, props: { message: build({}, { attachments: [shotAtt(1)] }), serverId: 'S1', onBack: () => {} } })
    await flush()
    document.querySelector<HTMLButtonElement>('.build-actions button[aria-haspopup=menu]')!.click()
    flushSync()
    ;[...document.querySelectorAll<HTMLElement>('.build-actions [role=menuitem]')].find((e) => e.textContent?.trim() === 'Edit')!.click()
    await flush()
    expect(q<HTMLInputElement>('input[aria-label="Title"]').value).toBe('Raven bunny')
    expect(q<HTMLInputElement>('input[aria-label="Game"]').value).toBe('The First Descendant')
    expect(q<HTMLInputElement>('input[aria-label="Build link"]').value).toBe('https://www.youtube.com/watch?v=dQw4w9WgXcQ')
    expect(q<HTMLTextAreaElement>('textarea[aria-label="Notes"]').value).toBe('Max crit')
    expect(shotNames()).toEqual(['shot1.webp'])
    typeIn(q<HTMLInputElement>('input[aria-label="Title"]'), 'Raven bunny v2')
    save().click()
    await flush()
    expect(calls('update_build')[0][1]).toEqual({
      messageId: 'M001', channelId: 'B1',
      body: { title: 'Raven bunny v2', game_id: 'G1', game: { set: 'G1' }, link_url: 'https://www.youtube.com/watch?v=dQw4w9WgXcQ', notes: 'Max crit', attachments: [], mark_sorted: false },
      order: ['id:A1'],
    })
    expect(document.querySelector('.build-form')).toBeNull()
  })

  it('editing a build with a guessed game sends it unchanged until it is kept', async () => {
    const guessed = build({ quick_drop: true, game_id: 'G1', game_guessed: true, notes: null })
    type Body = { game_id: string | null; game?: unknown; mark_sorted: boolean }
    const body = () => (calls('update_build')[0][1] as { body: Body }).body
    await form({ build: guessed })
    // the guess shows in the box, marked as guessed, with Keep
    expect(q<HTMLInputElement>('input[aria-label="Game"]').value).toBe('The First Descendant')
    expect(q('.guess').textContent).toContain('guessed')
    expect(q('.guess button.keep')).not.toBeNull()
    // still a quick drop with only a guess: Mark as sorted is offered
    expect(document.querySelector('.build-form input[name=sorted]')).not.toBeNull()
    save().click()
    await flush()
    // an untouched guess is sent as "unchanged": the server keeps it a guess (and the quick drop)
    expect(body()).toMatchObject({ game_id: 'G1', game: 'unchanged', mark_sorted: false })
    unmount(cmp!)
    invoke.mockClear()

    // Keep confirms it
    await form({ build: guessed })
    q<HTMLButtonElement>('.guess button.keep').click()
    flushSync()
    expect(document.querySelector('.build-form .guess')).toBeNull()
    expect(document.querySelector('.build-form input[name=sorted]')).toBeNull()
    expect(q<HTMLInputElement>('input[aria-label="Game"]').value).toBe('The First Descendant')
    save().click()
    await flush()
    expect(body()).toMatchObject({ game_id: 'G1', game: { set: 'G1' } })
    unmount(cmp!)
    invoke.mockClear()

    // clearing the box clears it
    await form({ build: guessed })
    typeIn(q<HTMLInputElement>('input[aria-label="Game"]'), '')
    expect(document.querySelector('.build-form .guess')).toBeNull()
    save().click()
    await flush()
    expect(body()).toMatchObject({ game_id: null, game: 'clear' })
  })

  it('cancelling after a failed save gives up its uploads', async () => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'stage_attachment_bytes') return staged('pasted.webp')
      if (cmd === 'create_build' || cmd === 'update_build') throw 'can\'t reach the server'
      return undefined
    })
    await form()
    typeIn(q<HTMLInputElement>('input[aria-label="Title"]'), 'Raven')
    paste(q('textarea[aria-label="Notes"]'), { files: [png()] })
    await flush()
    save().click()
    await flush()
    expect(q('.err').textContent).toContain('reach the server')
    const nonce = (calls('create_build')[0][1] as { body: { nonce: string } }).body.nonce
    ;[...document.querySelectorAll<HTMLButtonElement>('.build-form .foot button')].find((b) => b.textContent === 'Cancel')!.click()
    await flush()
    expect(calls('cancel_build_save').map((c) => c[1])).toEqual([{ nonce }])
    expect(calls('unstage_attachment').map((c) => c[1])).toEqual([{ id: 'st1' }])
    unmount(cmp!)
    invoke.mockClear()
    // an edit's save goes by the build's id
    await form({ build: build() })
    save().click()
    await flush()
    ;[...document.querySelectorAll<HTMLButtonElement>('.build-form .foot button')].find((b) => b.textContent === 'Cancel')!.click()
    await flush()
    expect(calls('cancel_build_save').map((c) => c[1])).toEqual([{ nonce: 'build-edit:M001' }])
  })

  it('Mark as sorted clears the quick drop badge', async () => {
    const drop = build({ quick_drop: true, game_id: null, notes: null })
    await form({ build: drop })
    const sorted = q<HTMLInputElement>('input[type=checkbox][name=sorted]')
    expect(sorted.closest('label')?.textContent).toContain('Mark as sorted')
    sorted.click()
    flushSync()
    save().click()
    await flush()
    expect((calls('update_build')[0][1] as { body: { mark_sorted: boolean } }).body.mark_sorted).toBe(true)
    // a build that isn't a quick drop has no such box
    unmount(cmp!)
    await form({ build: build() })
    expect(document.querySelector('.build-form input[name=sorted]')).toBeNull()
  })
})
