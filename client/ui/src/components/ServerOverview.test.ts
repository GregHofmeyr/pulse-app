// @vitest-environment jsdom
// Server settings › Overview (#55; the banner from #475 moved in): its owner changes the icon (the avatar's square
// crop), the banner (the profile banner's crop) and the name; everyone else sees how it looks. Then the invite card and
// the Invite dialog showing the banner, and the gradient when there's none.
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
const tab = () => [...document.querySelectorAll<HTMLButtonElement>('nav button')].find((b) => b.textContent === 'Overview')

/** Pick `file` in the hidden input labelled `label`, then apply the crop it opens. */
async function pickAndCrop(label: string) {
  const input = document.querySelector<HTMLInputElement>(`input[type=file][aria-label="${label}"]`)!
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
  const title = crop.d.getAttribute('aria-label')
  crop.i.dispatchEvent(new Event('load'))
  flushSync()
  button('Apply', crop.d)!.click()
  flushSync()
  return title
}

describe('Server settings › Overview (#55)', () => {
  it('is the first page, for everyone; the Banner page is gone', () => {
    openSettings(server({ owner_id: 'U2' }))
    const names = [...document.querySelectorAll<HTMLButtonElement>('nav button')].map((b) => b.textContent)
    expect(names[0]).toBe('Overview')
    expect(names).not.toContain('Banner')
    expect(document.querySelector('.content h2')!.textContent).toBe('Overview')
  })

  it("someone else's server: how it looks, nothing to change", () => {
    openSettings(server({ owner_id: 'U2', banner_hash: BN }))
    expect(document.querySelector<HTMLImageElement>('.content .server-banner img')!.src).toContain(BN)
    expect(document.querySelector('[aria-label="Change icon"]')).toBeNull()
    expect(document.querySelector('[aria-label="Change banner"]')).toBeNull()
    expect(document.querySelector('input[aria-label="Server name"]')).toBeNull()
    expect(document.querySelector('.content .hint')!.textContent).toContain('Only its owner')
  })

  it('shows the gradient and initial with neither picture, and both Removes are off', () => {
    openSettings(server())
    tab()!.click()
    flushSync()
    expect(document.querySelector('.content .server-banner img')).toBeNull()
    expect(document.querySelector('.content .icon span.chip')!.textContent).toBe('A')
    expect(button('Remove icon')!.disabled).toBe(true)
    expect(button('Remove banner')!.disabled).toBe(true)
  })

  it('the icon area opens the square crop, then uploads as an avatar and sets it', async () => {
    handlers.upload_avatar_media = () => ({ hash: BN, content_type: 'image/webp', width: 256, height: 256, frames: 1, animated: false, still_hash: null })
    handlers.set_server_icon = () => server({ icon_hash: BN })
    openSettings(server())
    const area = document.querySelector<HTMLButtonElement>('button[aria-label="Change icon"]')!
    const input = document.querySelector<HTMLInputElement>('input[aria-label="Upload icon"]')!
    const clicked = vi.spyOn(input, 'click').mockImplementation(() => {})
    area.click()
    expect(clicked).toHaveBeenCalled()
    expect(area.querySelector('.pencil')).not.toBeNull()
    expect(await pickAndCrop('Upload icon')).toBe('Crop avatar')
    await vi.waitFor(() => {
      if (!calls('set_server_icon').length) throw new Error('not set yet')
    })
    expect(calls('set_server_icon')).toEqual([{ serverId: 'S1', hash: BN }])
  })

  it('the banner area crops as a banner, then sets it', async () => {
    handlers.upload_banner_media = () => ({ hash: BN, content_type: 'image/webp', width: 960, height: 384, frames: 1, animated: false, still_hash: null })
    handlers.set_server_banner = () => server({ banner_hash: BN })
    openSettings(server())
    expect(await pickAndCrop('Upload banner')).toBe('Crop banner')
    await vi.waitFor(() => {
      if (!calls('set_server_banner').length) throw new Error('not set yet')
    })
    expect(invoke.mock.calls.some((c) => c[0] === 'upload_banner_media')).toBe(true)
    expect(calls('set_server_banner')).toEqual([{ serverId: 'S1', hash: BN }])
  })

  it('shows the pictures it has, and Remove clears each', async () => {
    openSettings(server({ banner_hash: BN, icon_hash: ST }))
    expect(document.querySelector<HTMLImageElement>('.content .server-banner img')!.src).toContain(BN)
    expect(document.querySelector<HTMLImageElement>('.content .icon img.chip')!.src).toContain(ST)
    button('Remove banner')!.click()
    await settle()
    expect(calls('set_server_banner')).toEqual([{ serverId: 'S1', hash: null }])
    button('Remove icon')!.click()
    await settle()
    expect(calls('set_server_icon')).toEqual([{ serverId: 'S1', hash: null }])
  })

  it('renames it with Save or Enter, trimmed; Save is off until the name changes', async () => {
    handlers.rename_server = () => server({ name: 'Lounge' })
    openSettings(server())
    const field = document.querySelector<HTMLInputElement>('input[aria-label="Server name"]')!
    expect(field.value).toBe('Alpha')
    expect(button('Save')!.disabled).toBe(true)
    field.value = '  Lounge '
    field.dispatchEvent(new Event('input', { bubbles: true }))
    flushSync()
    expect(button('Save')!.disabled).toBe(false)
    button('Save')!.click()
    await settle()
    expect(calls('rename_server')).toEqual([{ serverId: 'S1', name: 'Lounge' }])
    field.value = 'Beta'
    field.dispatchEvent(new Event('input', { bubbles: true }))
    flushSync()
    field.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    await settle()
    expect(calls('rename_server')).toEqual([{ serverId: 'S1', name: 'Lounge' }, { serverId: 'S1', name: 'Beta' }])
  })

  it('says why when it fails', async () => {
    handlers.set_server_banner = () => Promise.reject({ message: 'Only the server\'s owner can change its banner.' })
    openSettings(server({ banner_hash: BN }))
    button('Remove banner')!.click()
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
