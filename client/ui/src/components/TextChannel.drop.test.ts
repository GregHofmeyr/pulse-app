// @vitest-environment jsdom
// Files dropped on a chat (#57, test from #297): a drop whose point is on this chat queues them in its composer; one
// elsewhere in the window, or with a dialog open, doesn't.
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => []) }))
type DragDrop = { payload: { type: 'enter' | 'over' | 'leave' | 'drop'; position?: { x: number; y: number }; paths?: string[] } }
let onDrag: ((e: DragDrop) => void) | null = null
vi.mock('@tauri-apps/api/webview', () => ({
  getCurrentWebview: () => ({
    onDragDropEvent: async (f: (e: DragDrop) => void) => {
      onDrag = f
      return () => (onDrag = null)
    },
  }),
}))

import { invoke } from '@tauri-apps/api/core'
import TextChannel from './TextChannel.svelte'
import { app, resetState, ui } from '../lib/store.svelte'
import { PENDING_MS } from '../lib/clipDrop.svelte'
import { applyReady, emptyState, markHistory } from '../lib/state'
import { testReady } from '../lib/testReady'

let c: ReturnType<typeof mount> | undefined
/** What the point of a drag is over. */
let under: () => Element | null = () => null
const at = { x: 10, y: 10 }
const staged = () => vi.mocked(invoke).mock.calls.filter((call) => call[0] === 'stage_attachment_paths')

async function open() {
  c = mount(TextChannel, { target: document.body, props: { channelId: 'C1', serverId: 'S1' } })
  flushSync()
  for (let i = 0; i < 3; i++) await tick()
}
async function drop(paths: string[]) {
  onDrag!({ payload: { type: 'enter', position: at } })
  flushSync()
  onDrag!({ payload: { type: 'drop', position: at, paths } })
  for (let i = 0; i < 3; i++) await tick()
}

beforeEach(() => {
  Element.prototype.scrollTo = () => {}
  document.elementFromPoint = (() => under()) as typeof document.elementFromPoint
  vi.mocked(invoke).mockClear()
  resetState()
  app.state = applyReady(emptyState(), testReady([{ id: 'C1', server_id: 'S1', kind: 'text', name: 'general', position: 0 }]))
  app.state = markHistory(app.state, 'C1', true)
})
afterEach(() => {
  if (c) unmount(c)
  c = undefined
  document.body.innerHTML = ''
})

describe('TextChannel: dropping files', () => {
  it('a drop on the chat queues its files in the message box', async () => {
    await open()
    under = () => document.querySelector('.chan [role="textbox"]')
    expect(onDrag).not.toBeNull()
    onDrag!({ payload: { type: 'enter', position: at } })
    flushSync()
    // it lights up while files are over it
    expect(document.querySelector('.drop-zone')).not.toBeNull()
    await drop(['/home/alex/report.pdf', '/home/alex/shot.png'])
    expect(staged()).toHaveLength(1)
    expect(staged()[0][1]).toEqual({ paths: ['/home/alex/report.pdf', '/home/alex/shot.png'] })
    expect(document.querySelector('.drop-zone')).toBeNull()
  })

  it('a drop elsewhere in the window, or with a dialog open, queues nothing', async () => {
    await open()
    const outside = document.createElement('div')
    document.body.append(outside)
    under = () => outside
    onDrag!({ payload: { type: 'over', position: at } })
    flushSync()
    expect(document.querySelector('.drop-zone')).toBeNull()
    await drop(['/home/alex/report.pdf'])
    expect(staged()).toHaveLength(0)
    under = () => document.querySelector('.chan [role="textbox"]')
    const dialog = document.createElement('div')
    dialog.setAttribute('aria-modal', 'true')
    document.body.append(dialog)
    await drop(['/home/alex/report.pdf'])
    expect(staged()).toHaveLength(0)
  })

  // #394: an offered recording sent in a DM arrives as a request for this chat's composer
  it("an offered recording sent here is queued in the message box; another chat's, or a stale one, isn't", async () => {
    await open()
    ui.attachFiles = { channelId: 'C9', paths: ['/v/elsewhere.mp4'], at: Date.now() }
    flushSync()
    for (let i = 0; i < 3; i++) await tick()
    expect(staged()).toHaveLength(0)
    expect(ui.attachFiles).not.toBeNull()
    ui.attachFiles = { channelId: 'C1', paths: ['/v/old.mp4'], at: Date.now() - PENDING_MS - 1000 }
    flushSync()
    for (let i = 0; i < 3; i++) await tick()
    expect(staged()).toHaveLength(0)
    expect(ui.attachFiles).toBeNull()
    ui.attachFiles = { channelId: 'C1', paths: ['/v/Apex/new.mp4'], at: Date.now() }
    flushSync()
    for (let i = 0; i < 3; i++) await tick()
    expect(staged()).toHaveLength(1)
    expect(staged()[0][1]).toEqual({ paths: ['/v/Apex/new.mp4'] })
    expect(ui.attachFiles).toBeNull()
  })
})
