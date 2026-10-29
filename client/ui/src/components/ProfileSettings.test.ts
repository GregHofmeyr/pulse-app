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

const alex: User = { id: 'U1', username: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const saved = (over: Partial<Profile> = {}): Profile => ({
  user: alex,
  card: { banner_hash: null, banner_still: null, theme_top: '#3a3d46', theme_bottom: '#16171b', pronouns: 'they/them', bio: 'hello' },
  stats: { voice_seconds: 0, member_since: '2026-10-03T00:00:00Z', top_reaction: null, times_renamed: 0 },
  badges: [],
  name_archive: [],
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
const setColor = (el: HTMLInputElement, v: string) => { el.value = v; el.dispatchEvent(new Event('input', { bubbles: true })); flushSync() }
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

    const note = 'Lightened so it reads on dark backgrounds.'
    click(option('Name colour', 'Custom'))
    expect(document.body.textContent).not.toContain(note)
    const pick = document.querySelector<HTMLInputElement>('.colour input[type=color]')!
    setColor(pick, '#000000')
    expect(previewName().getAttribute('style')).toContain('color: rgb(168, 168, 168)')
    expect(document.body.textContent).toContain(note)
    click(option('Custom colour', 'Gradient'))
    const pickers = document.querySelectorAll<HTMLInputElement>('.colour input[type=color]')
    expect(pickers).toHaveLength(2)
    setColor(pickers[1], '#0000ff')
    expect(previewName().getAttribute('style')).toContain('linear-gradient(90deg, rgb(168, 168, 168), rgb(161, 161, 255))')
    setColor(pickers[0], '#ffc4a3')
    setColor(pickers[1], '#a3d8ff')
    expect(document.body.textContent).not.toContain(note)
  })

  it('picking a font and gradient restyles the preview name', async () => {
    await open()
    const fonts = [...group('Name font').querySelectorAll<HTMLButtonElement>('[role=radio]')]
    expect(fonts.map((b) => b.textContent!.trim())).toEqual(['Default', 'Elegant', 'Rounded', 'Pixel', 'Script', 'Handwritten', 'Grotesque', 'Quirky', 'Techy'])
    // each label is drawn in its own face
    expect(fonts[3].getAttribute('style')).toContain('Press Start 2P')
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
    expect(document.querySelector<HTMLInputElement>('input[aria-label="Pronouns"]')!.maxLength).toBe(40)
    click(button('Save'))
    click(button('Save')) // a second click while saving sends nothing
    expect(calls('update_profile')).toHaveLength(1)
    const req = (calls('update_profile')[0][1] as { request: UpdateProfileRequest }).request
    expect(req).toEqual({
      avatar_hash: null, banner_hash: null, theme_top: '#1f7a5a', theme_bottom: '#0f2b3a',
      style: { font: 'fredoka', colors: ['#000000'] }, pronouns: null, bio: 'hello',
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
