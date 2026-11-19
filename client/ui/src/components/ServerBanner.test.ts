// @vitest-environment jsdom
// Server banners (#475), the app side: Server settings › Banner (its owner's only: upload through the crop, then set;
// Remove), the invite card and the Invite dialog showing it, and the gradient when there's none.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fakeMatchMedia, FakeIntersectionObserver } from '../lib/testMotion'

type Handler = (args: unknown, opts?: unknown) => unknown
let handlers: Record<string, Handler> = {}
const invoke = vi.fn(async (cmd: string, args?: unknown, opts?: unknown): Promise<unknown> => handlers[cmd]?.(args, opts))
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never, opts?: never) => invoke(cmd, args, opts) }))
vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)

import ServerSettings from './ServerSettings.svelte'
import ServerInviteCard from './ServerInviteCard.svelte'
import InviteDialog from './InviteDialog.svelte'
import { app, resetState } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import type { Server } from '../lib/protocol/Server'
import type { InvitePreview } from '../lib/protocol/InvitePreview'

const BN = 'b'.repeat(64)
const ST = 'c'.repeat(64)
const alex = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const server = (over: Partial<Server> = {}): Server => ({ id: 'S1', name: 'Alpha', icon_hash: null, owner_id: 'U1', ...over })

let cmp: ReturnType<typeof mount> | undefined
const settle = async () => {
  for (let i = 0; i < 6; i++) await Promise.resolve()
  flushSync()
}
const calls = (cmd: string) => invoke.mock.calls.filter((c) => c[0] === cmd).map((c) => c[1])
const button = (text: string, root: ParentNode = document) => [...root.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.trim() === text)

beforeEach(() => {
  resetState()
  handlers = {}
  invoke.mockClear()
  FakeIntersectionObserver.all = []
  app.state = { ...emptyState(), me: alex, servers: [server()] } as never
  Object.defineProperty(HTMLImageElement.prototype, 'naturalWidth', { configurable: true, get: () => 800 })
  Object.defineProperty(HTMLImageElement.prototype, 'naturalHeight', { configurable: true, get: () => 600 })
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

function openSettings(s: Server) {
  cmp = mount(ServerSettings, { target: document.body, props: { server: s, onClose: () => {} } })
  flushSync()
}
const tab = () => [...document.querySelectorAll<HTMLButtonElement>('nav button')].find((b) => b.textContent === 'Banner')

describe('Server settings › Banner', () => {
  it("is its owner's only", () => {
    openSettings(server({ owner_id: 'U2' }))
    expect(tab()).toBeUndefined()
  })

  it('shows the gradient with no banner, and Remove is off', () => {
    openSettings(server())
    tab()!.click()
    flushSync()
    expect(document.querySelector('.content h2')!.textContent).toBe('Banner')
    expect(document.querySelector('.content .server-banner img')).toBeNull()
    expect(button('Remove')!.disabled).toBe(true)
  })

  it('uploads through the crop as a banner, then sets it', async () => {
    handlers.upload_banner_media = () => ({ hash: BN, content_type: 'image/webp', width: 960, height: 384, frames: 1, animated: false, still_hash: null })
    handlers.set_server_banner = () => server({ banner_hash: BN })
    openSettings(server())
    tab()!.click()
    flushSync()
    const input = document.querySelector<HTMLInputElement>('input[type=file][aria-label="Upload banner"]')!
    expect(input.accept).toBe('image/png,image/jpeg,image/gif,image/webp')
    Object.defineProperty(input, 'files', { configurable: true, value: [new File([new Uint8Array([1, 2, 3])], 'a.png', { type: 'image/png' })] })
    input.dispatchEvent(new Event('change', { bubbles: true }))
    flushSync()
    const crop = await vi.waitFor(() => {
      const d = document.querySelector<HTMLElement>('[role=dialog][aria-label^="Crop"]')
      const i = d?.querySelector<HTMLImageElement>('img')
      if (!d || !i) throw new Error('no crop image yet')
      return { d, i }
    })
    crop.i.dispatchEvent(new Event('load'))
    flushSync()
    button('Apply', crop.d)!.click()
    flushSync()
    await vi.waitFor(() => {
      if (!calls('set_server_banner').length) throw new Error('not set yet')
    })
    expect(invoke.mock.calls.some((c) => c[0] === 'upload_banner_media')).toBe(true)
    expect(calls('set_server_banner')).toEqual([{ serverId: 'S1', hash: BN }])
  })

  it('shows the banner it has, and Remove clears it', async () => {
    openSettings(server({ banner_hash: BN }))
    tab()!.click()
    flushSync()
    expect(document.querySelector<HTMLImageElement>('.content .server-banner img')!.src).toContain(BN)
    button('Remove')!.click()
    await settle()
    expect(calls('set_server_banner')).toEqual([{ serverId: 'S1', hash: null }])
  })

  it('says why when it fails', async () => {
    handlers.set_server_banner = () => Promise.reject({ message: 'Only the server\'s owner can change its banner.' })
    openSettings(server({ banner_hash: BN }))
    tab()!.click()
    flushSync()
    button('Remove')!.click()
    await settle()
    expect(document.querySelector('.content [role=alert]')!.textContent).toContain('owner can change its banner')
  })
})

describe('invite cards and the Invite dialog', () => {
  const preview = (over: Partial<InvitePreview> = {}): InvitePreview =>
    ({ code: 'abcDEF2345', server_id: 'S2', name: 'Beta', member_count: 2, online_count: 1, faces: [], inviter_id: 'U2', inviter_name: 'sam', state: 'valid', ...over })
  const card = async (p: InvitePreview) => {
    handlers.invite_preview = () => p
    cmp = mount(ServerInviteCard, { target: document.body, props: { code: p.code } })
    flushSync()
    FakeIntersectionObserver.set(document.querySelector('[data-server-invite]')!, true)
    await settle()
  }

  it("the card shows the server's banner (its still until it may move)", async () => {
    await card(preview({ code: 'bannerCd23', banner_hash: BN, banner_still: ST }))
    const img = document.querySelector<HTMLImageElement>('[data-server-invite] .server-banner img')!
    expect(img).not.toBeNull()
    expect(img.src).toMatch(new RegExp(`(${BN}|${ST})$`))
  })

  it('the card falls back to the gradient', async () => {
    await card(preview({ code: 'noBanner23' }))
    expect(document.querySelector('[data-server-invite] .server-banner')).not.toBeNull()
    expect(document.querySelector('[data-server-invite] .server-banner img')).toBeNull()
  })

  it("the Invite dialog shows the server's banner, or its glow", () => {
    handlers.server_origin = () => 'https://example.test'
    app.state = { ...app.state, servers: [server({ banner_hash: BN })] } as never
    cmp = mount(InviteDialog, { target: document.body, props: { serverId: 'S1', onClose: () => {} } })
    flushSync()
    expect(document.querySelector<HTMLImageElement>('[data-invite-dialog] .glow.pic img')!.src).toContain(BN)
    unmount(cmp)
    app.state = { ...app.state, servers: [server()] } as never
    cmp = mount(InviteDialog, { target: document.body, props: { serverId: 'S1', onClose: () => {} } })
    flushSync()
    expect(document.querySelector('[data-invite-dialog] .glow.pic')).toBeNull()
    expect(document.querySelector('[data-invite-dialog] .glow')).not.toBeNull()
  })
})
