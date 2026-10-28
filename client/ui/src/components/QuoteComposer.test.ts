// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message } from '../lib/protocol/Message'
import type { QuoteLine } from '../lib/protocol/QuoteLine'
import { fakeMatchMedia, FakeIntersectionObserver } from '../lib/testMotion'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))
// Files dropped on the window come as Tauri drag-drop events (paths and a physical position), never as HTML5 drops.
type DragHandler = (e: { payload: unknown }) => void
const drag = vi.hoisted(() => ({ handlers: [] as ((e: { payload: unknown }) => void)[] }))
vi.mock('@tauri-apps/api/webview', () => ({
  getCurrentWebview: () => ({
    onDragDropEvent: async (fn: DragHandler) => {
      drag.handlers.push(fn)
      return () => { drag.handlers = drag.handlers.filter((h) => h !== fn) }
    },
  }),
}))
vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)

import QuoteComposer from './QuoteComposer.svelte'
import { app } from '../lib/store.svelte'
import { applyEvent, emptyState, holdThread } from '../lib/state'

const alex = { id: 'U1', username: 'alex', avatar_hash: null }
const sam = { id: 'U2', username: 'sam', avatar_hash: null }
const kim = { id: 'U3', username: 'kim', avatar_hash: null }
const H = 'a'.repeat(64)
const S = 'b'.repeat(64)

let cmp: ReturnType<typeof mount> | undefined
const onDone = vi.fn()
function show(props: Record<string, unknown> = {}) {
  app.state = {
    ...emptyState(), me: alex, servers: [{ id: 'S1', name: 'Main', icon_hash: null }],
    members: { S1: [{ user: alex, nickname: null }, { user: sam, nickname: null }, { user: kim, nickname: null }] },
    people: { U1: { user: alex, online: true, last_seen_at: null }, U2: { user: sam, online: true, last_seen_at: null }, U3: { user: kim, online: true, last_seen_at: null } },
  }
  cmp = mount(QuoteComposer, { target: document.body, props: { channelId: 'C1', serverId: 'S1', onDone, ...props } })
  flushSync()
}
const settle = () => new Promise((r) => setTimeout(r, 0))
const calls = (cmd: string) => invoke.mock.calls.filter((c) => c[0] === cmd)
const rows = () => [...document.querySelectorAll<HTMLElement>('.qline')]
const type = (el: HTMLInputElement | HTMLTextAreaElement, v: string) => { el.value = v; el.dispatchEvent(new Event('input', { bubbles: true })); flushSync() }
const textOf = (n: number) => rows()[n].querySelector<HTMLTextAreaElement>('textarea.qtext')!
const whoOf = (n: number) => rows()[n].querySelector<HTMLInputElement>('input.who')!
const pickMember = (n: number, name: string) => {
  const w = whoOf(n)
  w.focus(); type(w, name)
  ;[...rows()[n].querySelectorAll<HTMLElement>('[role=option]')].find((o) => o.textContent?.trim() === name)!.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, cancelable: true }))
  ;[...rows()[n].querySelectorAll<HTMLElement>('[role=option]')].find((o) => o.textContent?.trim() === name)?.click()
  flushSync()
}
const enter = (el: HTMLElement) => { el.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true })); flushSync() }
const addLine = () => { document.querySelector<HTMLButtonElement>('button.addline')!.click(); flushSync() }
const imageFile = () => new File([new Uint8Array([1, 2, 3])], 'p.png', { type: 'image/png' })
const paste = (el: HTMLElement, f: File) => {
  const ev = Object.assign(new Event('paste', { bubbles: true, cancelable: true }), { clipboardData: { files: [f], items: [], getData: () => '' } })
  el.dispatchEvent(ev); flushSync()
}

// What is under the pointer (jsdom lays nothing out), and the points asked about.
let under: Element | null = null
const pointed: [number, number][] = []
Object.defineProperty(document, 'elementFromPoint', { configurable: true, value: (x: number, y: number) => { pointed.push([x, y]); return under } })
const CAT = '/home/alex/Pictures/cat.png'
const dragTo = (el: Element | null, type: 'enter' | 'over', position = { x: 10, y: 10 }, paths = [CAT]) => {
  under = el
  for (const h of [...drag.handlers]) h({ payload: type === 'enter' ? { type, paths, position } : { type, position } })
  flushSync()
}
const fireDrag = (payload: Record<string, unknown>) => { for (const h of [...drag.handlers]) h({ payload }); flushSync() }
async function dropOn(el: Element | null, paths = [CAT], position = { x: 10, y: 10 }) {
  dragTo(el, 'enter', position, paths)
  dragTo(el, 'over', position)
  fireDrag({ type: 'drop', paths, position })
  await settle(); await settle(); flushSync()
}
const lit = () => rows().map((r) => r.classList.contains('drop'))

beforeEach(() => {
  invoke.mockReset(); invoke.mockResolvedValue(undefined); onDone.mockClear()
  under = null; pointed.length = 0
  Object.defineProperty(window, 'devicePixelRatio', { configurable: true, value: 1 })
})
afterEach(() => { if (cmp) unmount(cmp); cmp = undefined; document.body.innerHTML = '' })

const eline = (over: Partial<QuoteLine> = {}): QuoteLine => ({
  id: 'L1', speaker_id: 'U2', speaker_name: null, text: 'hello', media: null, caption: null, awards: 0, awarders: [], awarded: false, ...over,
})
const quoteMsg = (lines: QuoteLine[]): Message => ({
  id: 'M1', channel_id: 'C1', author_id: 'U1', kind: 'normal', content: '', reply_to_id: null,
  created_at: '2026-10-01T10:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null,
  quote: { lines, year: 2026, said_on: null, reply_count: 0, last_reply_id: null, replies_seen: null, wins: [] }, qoty_card: null, clip: null,
})

describe('QuoteComposer', () => {
  it("won't post without a speaker and says Who said it?", async () => {
    show()
    type(textOf(0), 'hello')
    enter(textOf(0))
    await settle()
    expect(calls('post_quote')).toHaveLength(0)
    expect(document.body.textContent).toContain('Who said it?')
  })

  it('adds and removes layers up to 10', () => {
    show()
    for (let i = 0; i < 12; i++) if (document.querySelector('button.addline')) addLine()
    expect(rows()).toHaveLength(10)
    expect(document.querySelector('button.addline')).toBeNull()
    rows()[3].querySelector<HTMLButtonElement>('button[aria-label="Remove line"]')!.click(); flushSync()
    expect(rows()).toHaveLength(9)
    expect(document.querySelector('button.addline')).not.toBeNull()
  })

  it('posts lines with speaker ids and plain names', async () => {
    show()
    type(textOf(0), 'hello there'); pickMember(0, 'sam')
    addLine()
    type(textOf(1), 'back in my day')
    const w = whoOf(1); w.focus(); type(w, 'Grandpa')
    const use = [...rows()[1].querySelectorAll<HTMLElement>('[role=option]')].find((o) => o.textContent?.includes('not on Pulse yet'))!
    expect(use.textContent).toContain('Use “Grandpa”')
    use.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, cancelable: true })); use.click(); flushSync()
    enter(textOf(1))
    await settle()
    const [c] = calls('post_quote')
    const a = c[1] as { channelId: string; body: { lines: unknown[]; today: string; nonce: string } }
    expect(a.channelId).toBe('C1')
    expect(a.body.lines).toEqual([
      { id: null, speaker_id: 'U2', speaker_name: null, text: 'hello there', media_hash: null, caption: null },
      { id: null, speaker_id: null, speaker_name: 'Grandpa', text: 'back in my day', media_hash: null, caption: null },
    ])
    expect(a.body.today).toMatch(/^\d{4}-\d{2}-\d{2}$/)
    expect(a.body.nonce).toBeTruthy()
    expect(onDone).toHaveBeenCalled()
  })

  it('strips typed quote marks', async () => {
    show()
    type(textOf(0), '“hello”'); pickMember(0, 'sam'); enter(textOf(0))
    await settle()
    const body = (calls('post_quote')[0][1] as { body: { lines: { text: string }[] } }).body
    expect(body.lines[0].text).toBe('hello')
  })

  it('pasting an image makes an image line', async () => {
    invoke.mockImplementation(async (cmd) => (cmd === 'upload_quote_media' ? { hash: H, content_type: 'image/webp', width: 10, height: 10, frames: 4, animated: true, still_hash: S } : undefined))
    show()
    paste(textOf(0), imageFile())
    await settle(); flushSync()
    expect(calls('upload_quote_media')).toHaveLength(1)
    expect(rows()[0].querySelector('textarea.qtext')).toBeNull()
    expect(rows()[0].querySelector<HTMLImageElement>('img.thumb')!.getAttribute('src')).toContain(S)
    pickMember(0, 'sam')
    type(rows()[0].querySelector<HTMLInputElement>('input.caption')!, 'a cat')
    enter(rows()[0].querySelector<HTMLInputElement>('input.caption')!)
    await settle()
    const body = (calls('post_quote')[0][1] as { body: { lines: unknown[] } }).body
    expect(body.lines).toEqual([{ id: null, speaker_id: 'U2', speaker_name: null, text: null, media_hash: H, caption: 'a cat' }])
  })

  it('a_rejected_image_keeps_the_line', async () => {
    invoke.mockImplementation(async (cmd) => { if (cmd === 'upload_quote_media') throw 'That image is too big (2 MB max).'; return undefined })
    show()
    type(textOf(0), 'keep me')
    paste(textOf(0), imageFile())
    await settle(); flushSync()
    expect(rows()[0].textContent).toContain('too big')
    expect(textOf(0).value).toBe('keep me')
    expect(rows()).toHaveLength(1)
    expect(calls('post_quote')).toHaveLength(0)
  })

  it('edit warns before a speaker change on an awarded line', async () => {
    show({ editing: quoteMsg([eline({ awards: 3, awarders: ['U1', 'U3', 'U2'] })]) })
    expect(textOf(0).value).toBe('hello')
    rows()[0].querySelector<HTMLElement>('.chip')!.click(); flushSync()
    pickMember(0, 'kim')
    enter(textOf(0)); await settle()
    expect(document.body.textContent).toContain('This line has 3 awards. Changing who said it removes them.')
    expect(calls('edit_quote')).toHaveLength(0)
    document.querySelector<HTMLButtonElement>('button.confirm')!.click(); await settle()
    const [c] = calls('edit_quote')
    expect(c[1]).toEqual({ messageId: 'M1', body: { lines: [{ id: 'L1', speaker_id: 'U3', speaker_name: null, text: 'hello', media_hash: null, caption: null }] } })
  })

  it('says 1 award in the singular', async () => {
    show({ editing: quoteMsg([eline({ awards: 1, awarders: ['U1'] })]) })
    rows()[0].querySelector<HTMLElement>('.chip')!.click(); flushSync()
    pickMember(0, 'kim'); enter(textOf(0)); await settle()
    expect(document.body.textContent).toContain('This line has 1 award. Changing who said it removes them.')
  })

  it('does not warn for a plain-name recase and Escape cancels', async () => {
    show({ editing: quoteMsg([eline({ speaker_id: null, speaker_name: 'Grandpa Joe', awards: 2, awarders: ['U1', 'U3'] })]) })
    const w = rows()[0].querySelector<HTMLElement>('.chip')!; w.click(); flushSync()
    const who = whoOf(0); who.focus(); type(who, 'grandpa  JOE')
    ;[...rows()[0].querySelectorAll<HTMLElement>('[role=option]')].find((o) => o.textContent?.includes('not on Pulse yet'))!.click(); flushSync()
    enter(textOf(0)); await settle()
    expect(document.body.textContent).not.toContain('Changing who said it')
    expect(calls('edit_quote')).toHaveLength(1)
    textOf(0).dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })); flushSync()
    expect(onDone).toHaveBeenCalled()
  })

  it('retries a failed post with the same nonce, then uses a fresh one', async () => {
    let n = 0
    invoke.mockImplementation(async (cmd) => { if (cmd === 'post_quote' && n++ === 0) throw 'network down'; return undefined })
    show()
    type(textOf(0), 'hi'); pickMember(0, 'sam'); enter(textOf(0)); await settle()
    expect(document.body.textContent).toContain('network down')
    enter(textOf(0)); await settle()
    const ns = calls('post_quote').map((c) => (c[1] as { body: { nonce: string } }).body.nonce)
    expect(ns).toHaveLength(2)
    expect(ns[0]).toBe(ns[1])
    type(textOf(0), 'again'); pickMember(0, 'sam'); enter(textOf(0)); await settle()
    expect((calls('post_quote')[2][1] as { body: { nonce: string } }).body.nonce).not.toBe(ns[0])
  })

  it('the picker is keyboard driven: ArrowDown reaches the plain-name option', async () => {
    show()
    type(textOf(0), 'hi')
    const w = whoOf(0); w.focus(); type(w, 'sa')
    const key = (k: string) => { w.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true })); flushSync() }
    const opts = () => [...rows()[0].querySelectorAll<HTMLElement>('[role=option]')]
    expect(opts().map((o) => o.getAttribute('aria-selected'))).toEqual(['true', 'false'])
    key('ArrowDown')
    expect(opts().map((o) => o.getAttribute('aria-selected'))).toEqual(['false', 'true'])
    expect(w.getAttribute('aria-activedescendant')).toBe(opts()[1].id)
    key('Enter')
    expect(rows()[0].querySelector('.chip')!.textContent).toContain('sa')
    expect(rows()[0].querySelector('.chip')!.textContent).not.toContain('sam')
  })

  it('the image button is a real button that opens the file chooser', () => {
    show()
    const b = rows()[0].querySelector<HTMLElement>('[aria-label="Add image"]')!
    expect(b.tagName).toBe('BUTTON')
    const input = rows()[0].querySelector<HTMLInputElement>('input[type=file]')!
    const click = vi.spyOn(input, 'click')
    b.click()
    expect(click).toHaveBeenCalled()
  })

  it('refocuses the text box after posting', async () => {
    show()
    type(textOf(0), 'hi'); pickMember(0, 'sam'); enter(textOf(0)); await settle()
    expect(document.activeElement).toBe(textOf(0))
    expect(textOf(0).value).toBe('')
  })

  const uploaded = { hash: H, content_type: 'image/webp', width: 10, height: 10, frames: 1, animated: false, still_hash: null }
  const dropped = new Uint8Array([7, 8, 9]).buffer
  const readsAndUploads = (cmd: string) => (cmd === 'read_quote_image' ? dropped : cmd === 'upload_quote_media' ? uploaded : undefined)
  it('a dropped image file is read by the core and uploads', async () => {
    invoke.mockImplementation(async (cmd) => readsAndUploads(cmd))
    show()
    await dropOn(textOf(0))
    expect(calls('read_quote_image')).toEqual([['read_quote_image', { paths: [CAT] }]])
    expect(calls('upload_quote_media')).toHaveLength(1)
    expect([...(calls('upload_quote_media')[0][1] as Uint8Array)]).toEqual([7, 8, 9])
    expect(rows()[0].querySelector('img.thumb')).not.toBeNull()
  })

  it('a drop lands on the line under the pointer, its position taken from physical to CSS pixels', async () => {
    invoke.mockImplementation(async (cmd) => readsAndUploads(cmd))
    Object.defineProperty(window, 'devicePixelRatio', { configurable: true, value: 1.5 })
    show()
    addLine()
    await dropOn(whoOf(1), [CAT], { x: 300, y: 120 })
    expect(pointed.at(-1)).toEqual([200, 80])
    expect(rows()[0].querySelector('img.thumb')).toBeNull()
    expect(rows()[1].querySelector('img.thumb')).not.toBeNull()
  })

  it('a drop off the lines does nothing', async () => {
    invoke.mockImplementation(async (cmd) => readsAndUploads(cmd))
    show()
    await dropOn(document.querySelector('button.post'))
    await dropOn(null)
    expect(calls('read_quote_image')).toHaveLength(0)
    expect(rows()[0].querySelector('.err')).toBeNull()
  })

  it('the line under a file drag lights up, and goes out on leave or drop', async () => {
    invoke.mockImplementation(async (cmd) => readsAndUploads(cmd))
    show()
    addLine()
    expect(lit()).toEqual([false, false])
    dragTo(textOf(0), 'enter')
    expect(lit()).toEqual([true, false])
    dragTo(whoOf(1), 'over')
    expect(lit()).toEqual([false, true])
    dragTo(document.querySelector('button.post'), 'over')
    expect(lit()).toEqual([false, false])
    dragTo(textOf(0), 'over')
    fireDrag({ type: 'leave' })
    expect(lit()).toEqual([false, false])
    await dropOn(textOf(1))
    expect(lit()).toEqual([false, false])
  })

  it('several files dropped go to the core together (it takes the first image)', async () => {
    invoke.mockImplementation(async (cmd) => readsAndUploads(cmd))
    show()
    const paths = ['/home/alex/notes.txt', CAT, '/home/alex/Pictures/dog.png']
    await dropOn(textOf(0), paths)
    expect(calls('read_quote_image')).toEqual([['read_quote_image', { paths }]])
    expect(calls('upload_quote_media')).toHaveLength(1)
  })

  it("a dropped file that isn't an image says so on its line, in the core's words, and uploads nothing", async () => {
    const words = "that isn't a supported image: use PNG, JPEG, GIF or WebP"
    invoke.mockImplementation(async (cmd) => { if (cmd === 'read_quote_image') throw words; return undefined })
    show()
    type(textOf(0), 'keep me')
    await dropOn(textOf(0), ['/home/alex/notes.txt'])
    expect(calls('upload_quote_media')).toHaveLength(0)
    expect(rows()[0].querySelector('.err')!.textContent).toBe(words)
    expect(textOf(0).value).toBe('keep me')
    expect(rows()[0].querySelector('.note')).toBeNull()
  })

  it('with two composers open (new quote and an edit), only the one under the drop takes it', async () => {
    invoke.mockImplementation(async (cmd) => readsAndUploads(cmd))
    show()
    const second = document.createElement('div')
    document.body.appendChild(second)
    const other = mount(QuoteComposer, { target: second, props: { channelId: 'C1', serverId: 'S1', editing: quoteMsg([eline()]) } })
    flushSync()
    await settle()
    expect(drag.handlers).toHaveLength(2)
    await dropOn(second.querySelector('textarea.qtext'))
    expect(calls('read_quote_image')).toHaveLength(1)
    expect(second.querySelector('img.thumb')).not.toBeNull()
    expect(rows()[0].querySelector('img.thumb')).toBeNull()
    unmount(other)
    expect(drag.handlers).toHaveLength(1)
  })

  it('a picked file uploads', async () => {
    invoke.mockImplementation(async (cmd) => (cmd === 'upload_quote_media' ? uploaded : undefined))
    show()
    const input = rows()[0].querySelector<HTMLInputElement>('input[type=file]')!
    Object.defineProperty(input, 'files', { value: [imageFile()], configurable: true })
    input.dispatchEvent(new Event('change', { bubbles: true })); await settle(); flushSync()
    expect(calls('upload_quote_media')).toHaveLength(1)
    expect(rows()[0].querySelector('img.thumb')).not.toBeNull()
  })

  it('typed words become the caption when an image arrives; too long keeps them and converts nothing', async () => {
    invoke.mockImplementation(async (cmd) => (cmd === 'upload_quote_media' ? uploaded : undefined))
    show()
    type(textOf(0), '  a cat  ')
    paste(textOf(0), imageFile()); await settle(); flushSync()
    expect(rows()[0].querySelector<HTMLInputElement>('input.caption')!.value).toBe('a cat')
    unmount(cmp!); document.body.innerHTML = ''; invoke.mockClear()
    show()
    type(textOf(0), 'x'.repeat(201))
    paste(textOf(0), imageFile()); await settle(); flushSync()
    expect(calls('upload_quote_media')).toHaveLength(0)
    expect(textOf(0).value).toHaveLength(201)
    expect(rows()[0].textContent).toContain('too long for a caption')
  })

  it('Enter while an image uploads says to wait and Post is disabled', async () => {
    let done: (v: unknown) => void = () => {}
    invoke.mockImplementation((cmd) => (cmd === 'upload_quote_media' ? new Promise((r) => (done = r)) : Promise.resolve(undefined)))
    show()
    addLine()
    type(textOf(0), 'hi'); pickMember(0, 'sam')
    paste(textOf(1), imageFile()); await settle(); flushSync()
    expect(document.querySelector<HTMLButtonElement>('button.post')!.disabled).toBe(true)
    enter(textOf(0)); await settle()
    expect(document.body.textContent).toContain('Wait for the image to finish uploading.')
    expect(calls('post_quote')).toHaveLength(0)
    done(uploaded); await settle()
  })

  it('a typed but unpicked name resolves to the member it names, else stays a plain name', async () => {
    show()
    type(textOf(0), 'hi')
    const w = whoOf(0); w.focus(); type(w, ' SAM ')
    enter(textOf(0)); await settle()
    expect((calls('post_quote')[0][1] as { body: { lines: { speaker_id: string | null }[] } }).body.lines[0].speaker_id).toBe('U2')
    type(textOf(0), 'yo')
    const w2 = whoOf(0); w2.focus(); type(w2, 'sa')
    enter(textOf(0)); await settle()
    const l = (calls('post_quote')[1][1] as { body: { lines: { speaker_id: string | null; speaker_name: string | null }[] } }).body.lines[0]
    expect(l).toMatchObject({ speaker_id: null, speaker_name: 'sa' })
  })

  it('counts a name in characters, not UTF-16 units', async () => {
    show()
    type(textOf(0), 'hi')
    const name = '😀'.repeat(32)
    const w = whoOf(0); w.focus(); type(w, name)
    enter(textOf(0)); await settle()
    expect((calls('post_quote')[0][1] as { body: { lines: { speaker_name: string }[] } }).body.lines[0].speaker_name).toBe(name)
    type(textOf(0), 'yo')
    const w2 = whoOf(0); w2.focus(); type(w2, '😀'.repeat(33))
    enter(textOf(0)); await settle()
    expect(calls('post_quote')).toHaveLength(1)
    expect(document.body.textContent).toContain('at most 32')
  })

  it('a name that is only dots is refused before sending, as the server would', async () => {
    show()
    type(textOf(0), 'hi')
    const w = whoOf(0); w.focus(); type(w, '..')
    enter(textOf(0)); await settle()
    expect(calls('post_quote')).toHaveLength(0)
    expect(document.body.textContent).toContain('A name needs more than dots.')
    // Focus goes to the line's who field (the chip, once a name is picked) to fix it.
    expect((document.activeElement as HTMLElement | null)?.matches('.who, .chip')).toBe(true)
    expect(document.activeElement?.closest('.row')).toBe(textOf(0).closest('.row'))
  })

  it('saving an edit with every line cleared says so', async () => {
    show({ editing: quoteMsg([eline()]) })
    type(textOf(0), '')
    rows()[0].querySelector<HTMLElement>('.chip')!.click(); flushSync()
    enter(textOf(0)); await settle()
    expect(document.body.textContent).toContain('Add the words or an image.')
    expect(calls('edit_quote')).toHaveLength(0)
  })

  it('several awarded changes name their lines', async () => {
    show({ editing: quoteMsg([eline({ awards: 2, awarders: ['U1', 'U3'], text: 'first' }), eline({ id: 'L2', awards: 1, awarders: ['U1'], text: 'second' })]) })
    for (const n of [0, 1]) { rows()[n].querySelector<HTMLElement>('.chip')!.click(); flushSync(); pickMember(n, 'kim') }
    enter(textOf(0)); await settle()
    const t = document.querySelector('.warn')!.textContent!
    expect(t).toContain('“first” This line has 2 awards. Changing who said it removes them.')
    expect(t).toContain('“second” This line has 1 award. Changing who said it removes them.')
  })

  // M18: a new image on an image line keeps its caption; pasting an image into the caption box replaces the image.
  it('a new image dropped on an image line keeps its caption', async () => {
    let n = 0
    invoke.mockImplementation(async (cmd) => (cmd === 'upload_quote_media' ? { ...uploaded, hash: n++ ? S : H } : undefined))
    show()
    type(textOf(0), 'a cat')
    paste(textOf(0), imageFile()); await settle(); flushSync()
    const cap = () => rows()[0].querySelector<HTMLInputElement>('input.caption')!
    expect(cap().value).toBe('a cat')
    await dropOn(rows()[0].querySelector('.row'))
    expect(calls('upload_quote_media')).toHaveLength(2)
    expect(rows()[0].querySelector<HTMLImageElement>('img.thumb')!.getAttribute('src')).toContain(S)
    expect(cap().value).toBe('a cat')
  })

  it('pasting an image into the caption box replaces the image and keeps the caption', async () => {
    let n = 0
    invoke.mockImplementation(async (cmd) => (cmd === 'upload_quote_media' ? { ...uploaded, hash: n++ ? S : H } : undefined))
    show()
    paste(textOf(0), imageFile()); await settle(); flushSync()
    const cap = () => rows()[0].querySelector<HTMLInputElement>('input.caption')!
    type(cap(), 'a dog')
    paste(cap(), imageFile()); await settle(); flushSync()
    expect(calls('upload_quote_media')).toHaveLength(2)
    expect(rows()[0].querySelector<HTMLImageElement>('img.thumb')!.getAttribute('src')).toContain(S)
    expect(cap().value).toBe('a dog')
  })

  it('a caption typed while a new image uploads is kept', async () => {
    let done: (v: unknown) => void = () => {}
    let n = 0
    invoke.mockImplementation((cmd) => (cmd === 'upload_quote_media' ? (n++ ? new Promise((r) => (done = r)) : Promise.resolve(uploaded)) : Promise.resolve(undefined)))
    show()
    type(textOf(0), 'a cat')
    paste(textOf(0), imageFile()); await settle(); flushSync()
    const cap = () => rows()[0].querySelector<HTMLInputElement>('input.caption')!
    await dropOn(rows()[0].querySelector('.row'))
    type(cap(), 'a cat, napping')
    done({ ...uploaded, hash: S }); await settle(); flushSync()
    expect(rows()[0].querySelector<HTMLImageElement>('img.thumb')!.getAttribute('src')).toContain(S)
    expect(cap().value).toBe('a cat, napping')
  })

  it('words typed while a text line\'s first image uploads become its caption', async () => {
    let done: (v: unknown) => void = () => {}
    invoke.mockImplementation((cmd) => (cmd === 'upload_quote_media' ? new Promise((r) => (done = r)) : Promise.resolve(undefined)))
    show()
    type(textOf(0), 'a cat')
    paste(textOf(0), imageFile()); await settle(); flushSync()
    type(textOf(0), 'a cat, napping')
    done(uploaded); await settle(); flushSync()
    expect(rows()[0].querySelector('img.thumb')).not.toBeNull()
    expect(rows()[0].querySelector<HTMLInputElement>('input.caption')!.value).toBe('a cat, napping')
  })

  it('an image removed while its replacement uploads stays removed and the typed words stay', async () => {
    let done: (v: unknown) => void = () => {}
    let n = 0
    invoke.mockImplementation((cmd) => (cmd === 'upload_quote_media' ? (n++ ? new Promise((r) => (done = r)) : Promise.resolve(uploaded)) : Promise.resolve(undefined)))
    show()
    paste(textOf(0), imageFile()); await settle(); flushSync()
    await dropOn(rows()[0].querySelector('.row'))
    rows()[0].querySelector<HTMLButtonElement>('button[aria-label="Remove image"]')!.click(); flushSync()
    type(textOf(0), 'never mind the picture')
    done({ ...uploaded, hash: S }); await settle(); flushSync()
    expect(rows()[0].querySelector('img.thumb')).toBeNull()
    expect(textOf(0).value).toBe('never mind the picture')
  })

  it('a caption over 200 characters is refused when posting', async () => {
    invoke.mockImplementation(async (cmd) => (cmd === 'upload_quote_media' ? uploaded : undefined))
    show()
    paste(textOf(0), imageFile()); await settle(); flushSync()
    pickMember(0, 'sam')
    const cap = rows()[0].querySelector<HTMLInputElement>('input.caption')!
    type(cap, 'x'.repeat(201))
    enter(cap); await settle()
    expect(calls('post_quote')).toHaveLength(0)
    expect(document.body.textContent).toContain('A caption can be at most 200 characters.')
    expect(cap.value).toHaveLength(201)
  })

  // M19: the speaker-change warning can't be bypassed.
  const twoAwarded = () => quoteMsg([eline({ awards: 2, awarders: ['U1', 'U3'], text: 'first' }), eline({ id: 'L2', awards: 1, awarders: ['U1'], text: 'second' })])
  const warning = () => document.querySelector('.warn')
  it('changing another line while the warning is up takes the warning down', async () => {
    show({ editing: twoAwarded() })
    rows()[0].querySelector<HTMLElement>('.chip')!.click(); flushSync(); pickMember(0, 'kim')
    enter(textOf(0)); await settle()
    expect(warning()).not.toBeNull()
    rows()[1].querySelector<HTMLElement>('.chip')!.click(); flushSync()
    expect(warning()).toBeNull()
  })

  const warnFor0 = async () => {
    rows()[0].querySelector<HTMLElement>('.chip')?.click(); flushSync(); pickMember(0, 'kim')
    enter(textOf(0)); await settle()
    expect(warning()).not.toBeNull()
  }
  it('a typed name resolving takes the warning down', async () => {
    show({ editing: twoAwarded() })
    // a blank third line is skipped, so the warning can be up while a name is typed into it
    addLine()
    await warnFor0()
    const w = whoOf(2); w.focus(); type(w, 'Grandpa'); w.dispatchEvent(new FocusEvent('blur')); flushSync()
    expect(rows()[2].querySelector('.chip')!.textContent).toContain('Grandpa')
    expect(warning()).toBeNull()
  })

  it('adding a line takes the warning down', async () => {
    show({ editing: twoAwarded() })
    await warnFor0()
    addLine()
    expect(warning()).toBeNull()
  })

  it('removing a line takes the warning down', async () => {
    show({ editing: twoAwarded() })
    addLine()
    await warnFor0()
    rows()[2].querySelector<HTMLButtonElement>('button[aria-label="Remove line"]')!.click(); flushSync()
    expect(warning()).toBeNull()
  })

  it('a line that gains its first award mid-edit warns', async () => {
    const m = quoteMsg([eline()])
    show({ editing: m })
    app.state = { ...app.state, messages: { C1: [m] } }
    rows()[0].querySelector<HTMLElement>('.chip')!.click(); flushSync(); pickMember(0, 'kim')
    app.state = { ...app.state, messages: { C1: [quoteMsg([eline({ awards: 1, awarders: ['U3'] })])] } }
    flushSync()
    enter(textOf(0)); await settle()
    expect(calls('edit_quote')).toHaveLength(0)
    expect(document.body.textContent).toContain('This line has 1 award. Changing who said it removes them.')
  })

  it('fix-wave re-review: editing a quote held only by its replies panel (it left the list) reads its live awards', async () => {
    const m = quoteMsg([eline()])
    show({ editing: m })
    // after a jump the channel's list no longer has the quote; the open panel holds it
    app.state = holdThread({ ...app.state, messages: { C1: [] } }, m)
    rows()[0].querySelector<HTMLElement>('.chip')!.click(); flushSync(); pickMember(0, 'kim')
    app.state = applyEvent(app.state, { t: 'QuoteAwardChanged', d: { channel_id: 'C1', message_id: 'M1', line_id: 'L1', user_id: 'U3', given: true, count: 1, awarders: ['U3'] } }, 0)
    flushSync()
    enter(textOf(0)); await settle()
    expect(calls('edit_quote')).toHaveLength(0)
    expect(document.body.textContent).toContain('This line has 1 award. Changing who said it removes them.')
  })

  it('"Save anyway" skips only the lines that were confirmed', async () => {
    const m = quoteMsg([eline({ text: 'first' }), eline({ id: 'L2', awards: 1, awarders: ['U1'], text: 'second' })])
    show({ editing: m })
    app.state = { ...app.state, messages: { C1: [m] } }
    for (const n of [0, 1]) { rows()[n].querySelector<HTMLElement>('.chip')!.click(); flushSync(); pickMember(n, 'kim') }
    enter(textOf(0)); await settle()
    expect(warning()!.textContent).not.toContain('“first”')
    // line 1 gains an award while the warning for line 2 is up
    app.state = { ...app.state, messages: { C1: [quoteMsg([eline({ text: 'first', awards: 1, awarders: ['U3'] }), eline({ id: 'L2', awards: 1, awarders: ['U1'], text: 'second' })])] } }
    flushSync()
    document.querySelector<HTMLButtonElement>('button.confirm')!.click(); await settle()
    expect(calls('edit_quote')).toHaveLength(0)
    expect(warning()!.textContent).toContain('“first” This line has 1 award.')
    document.querySelector<HTMLButtonElement>('button.confirm')!.click(); await settle()
    expect(calls('edit_quote')).toHaveLength(1)
  })

  it('a retry after changing the text gets a new nonce', async () => {
    let n = 0
    invoke.mockImplementation(async (cmd) => { if (cmd === 'post_quote' && n++ === 0) throw 'network down'; return undefined })
    show()
    type(textOf(0), 'hi'); pickMember(0, 'sam'); enter(textOf(0)); await settle()
    type(textOf(0), 'hi there'); enter(textOf(0)); await settle()
    const ns = calls('post_quote').map((c) => (c[1] as { body: { nonce: string } }).body.nonce)
    expect(ns).toHaveLength(2)
    expect(ns[0]).not.toBe(ns[1])
  })
})
