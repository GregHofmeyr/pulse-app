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

  /** Settings opened on the Profile section, loaded. */
  async function openSettings(p: Profile = saved()) {
    handlers.get_profile = () => p
    app.state = { ...emptyState(), me: alex, people: { U1: { user: alex, online: true, last_seen_at: null } } } as never
    const onClose = vi.fn()
    cmp = mount(Settings, { target: document.body, props: { onClose, section: 'profile' } })
    await vi.waitFor(() => {
      flushSync()
      if (!document.querySelector('.preview .card')) throw new Error('not loaded')
    })
    return onClose
  }
  const closeButton = () => document.querySelector<HTMLButtonElement>('button[aria-label="Close settings"]')!
  const settle = () => vi.waitFor(() => { flushSync() })

  it('edits made while a Save is on its way are kept on top of the answer', async () => {
    let answer!: (p: Profile) => void
    handlers.update_profile = () => new Promise<Profile>((r) => (answer = r))
    const state = await open()
    click(option('Name colour', 'Custom'))
    setColor(document.querySelector<HTMLInputElement>('.colour input[type=color]')!, '#000000')
    click(button('Save'))
    click(option('Card theme', 'Berry')) // while saving
    typeBio('typed meanwhile')
    answer(saved({ user: { ...alex, style: { font: null, colors: ['#a8a8a8'] } } }))
    await vi.waitFor(() => {
      flushSync()
      expect(button('Save').disabled).toBe(false)
    })
    expect(option('Card theme', 'Berry').getAttribute('aria-checked')).toBe('true')
    expect(bioBox().textContent).toBe('typed meanwhile')
    // what wasn't touched since takes the server's answer (the lightened colour)
    expect(document.querySelector<HTMLInputElement>('.colour input[type=color]')!.value).toBe('#a8a8a8')
    expect(document.body.textContent).toContain('Unsaved changes')
    expect(state.dirty).toBe(true)
    expect(app.state.profiles.U1.user.style.colors).toEqual(['#a8a8a8'])
  })

  it('the answer reaches the cache even if the editor has gone', async () => {
    let answer!: (p: Profile) => void
    handlers.update_profile = () => new Promise<Profile>((r) => (answer = r))
    await open()
    click(option('Card theme', 'Berry'))
    click(button('Save'))
    unmount(cmp!)
    cmp = undefined
    const stored = saved({ card: { ...saved().card, theme_top: '#c2457a', theme_bottom: '#3a1f5e' } })
    answer(stored)
    await vi.waitFor(() => expect(app.state.profiles.U1).toEqual(stored))
  })

  it('closing during a Save waits for it: no discard prompt when it goes through', async () => {
    let answer!: (p: Profile) => void
    handlers.update_profile = () => new Promise<Profile>((r) => (answer = r))
    const onClose = await openSettings()
    click(option('Card theme', 'Berry'))
    click(button('Save'))
    click(closeButton())
    await settle()
    expect(document.body.textContent).not.toContain('Discard your profile changes?')
    expect(onClose).not.toHaveBeenCalled()
    const stored = saved({ card: { ...saved().card, theme_top: '#c2457a', theme_bottom: '#3a1f5e' } })
    answer(stored)
    await vi.waitFor(() => expect(onClose).toHaveBeenCalledTimes(1))
    expect(document.body.textContent).not.toContain('Discard your profile changes?')
    expect(app.state.profiles.U1).toEqual(stored)
  })

  it('a second way out during a Save wins over the first, and shows Saving… until it settles', async () => {
    let answer!: (p: Profile) => void
    handlers.update_profile = () => new Promise<Profile>((r) => (answer = r))
    const onClose = await openSettings()
    const section = (label: string) => [...document.querySelectorAll<HTMLButtonElement>('nav button')].find((b) => b.textContent === label)!
    click(option('Card theme', 'Berry'))
    click(button('Save'))
    click(closeButton())
    await settle()
    expect(document.querySelector('.content [role=status]')!.textContent).toBe('Saving your profile…')
    expect(closeButton().getAttribute('aria-busy')).toBe('true')
    click(section('Chat'))
    await settle()
    // the latest request is the one shown as waiting
    expect(closeButton().hasAttribute('aria-busy')).toBe(false)
    expect(section('Chat').getAttribute('aria-busy')).toBe('true')
    answer(saved({ card: { ...saved().card, theme_top: '#c2457a', theme_bottom: '#3a1f5e' } }))
    await vi.waitFor(() => {
      flushSync()
      expect(section('Chat').getAttribute('aria-current')).toBe('page')
    })
    expect(onClose).not.toHaveBeenCalled()
    expect(document.querySelector('.content [role=status]')).toBeNull()
    expect(section('Chat').hasAttribute('aria-busy')).toBe(false)
  })

  it('asking for Profile again during a Save stays put', async () => {
    let answer!: (p: Profile) => void
    handlers.update_profile = () => new Promise<Profile>((r) => (answer = r))
    const onClose = await openSettings()
    const section = (label: string) => [...document.querySelectorAll<HTMLButtonElement>('nav button')].find((b) => b.textContent === label)!
    click(option('Card theme', 'Berry'))
    click(button('Save'))
    click(closeButton())
    click(section('Profile'))
    await settle()
    expect(document.querySelector('.content [role=status]')).toBeNull()
    answer(saved({ card: { ...saved().card, theme_top: '#c2457a', theme_bottom: '#3a1f5e' } }))
    await vi.waitFor(() => {
      flushSync()
      expect(document.body.textContent).toContain('Saved')
    })
    expect(onClose).not.toHaveBeenCalled()
    expect(section('Profile').getAttribute('aria-current')).toBe('page')
  })

  it('closing during a Save that fails asks, with the error shown', async () => {
    let refuse!: (e: unknown) => void
    handlers.update_profile = () => new Promise<Profile>((_, r) => (refuse = r))
    const onClose = await openSettings()
    click(option('Card theme', 'Berry'))
    click(button('Save'))
    click(closeButton())
    refuse('Colours must look like #a1b2c3.')
    await vi.waitFor(() => {
      flushSync()
      expect(document.body.textContent).toContain('Discard your profile changes?')
    })
    expect(document.querySelector('.footer [role=alert]')!.textContent).toBe('Colours must look like #a1b2c3.')
    expect(onClose).not.toHaveBeenCalled()
  })

  it('closing during an upload asks first', async () => {
    handlers.upload_avatar_media = () => new Promise<MediaInfo>(() => {})
    const onClose = await openSettings()
    await upload('Avatar')
    await vi.waitFor(() => {
      flushSync()
      expect(document.body.textContent).toContain('Uploading…')
    })
    click(closeButton())
    await settle()
    expect(document.body.textContent).toContain('Discard your profile changes?')
    expect(onClose).not.toHaveBeenCalled()
  })

  it('Escape while the discard prompt shows means Keep editing', async () => {
    const onClose = await openSettings()
    click(option('Card theme', 'Berry'))
    click(closeButton())
    await settle()
    expect(document.body.textContent).toContain('Discard your profile changes?')
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
    flushSync()
    expect(document.body.textContent).not.toContain('Discard your profile changes?')
    expect(onClose).not.toHaveBeenCalled()
    expect(option('Card theme', 'Berry').getAttribute('aria-checked')).toBe('true')
  })

  it('Reset during an upload drops it when it lands', async () => {
    let finish!: (m: MediaInfo) => void
    handlers.upload_avatar_media = () => new Promise<MediaInfo>((r) => (finish = r))
    const state = await open()
    await upload('Avatar')
    await vi.waitFor(() => {
      flushSync()
      expect(calls('upload_avatar_media')).toHaveLength(1)
    })
    expect(document.body.textContent).toContain('Uploading…')
    expect(state.dirty).toBe(true)
    click(button('Reset'))
    expect(document.body.textContent).not.toContain('Uploading…')
    finish(media('LATE'))
    await settle()
    await new Promise((r) => setTimeout(r, 0))
    flushSync()
    expect(previewLine().querySelector('img')).toBeNull()
    expect(state.dirty).toBe(false)
  })

  it('Remove during an upload cancels it', async () => {
    let finish!: (m: MediaInfo) => void
    handlers.upload_avatar_media = () => new Promise<MediaInfo>((r) => (finish = r))
    await open(saved({ user: { ...alex, avatar_hash: 'OLD' } }))
    await upload('Avatar')
    await vi.waitFor(() => {
      flushSync()
      expect(calls('upload_avatar_media')).toHaveLength(1)
    })
    click(button('Remove', document.querySelector('.avatar-field')!))
    finish(media('LATE'))
    await new Promise((r) => setTimeout(r, 0))
    flushSync()
    expect(previewLine().querySelector('img')).toBeNull()
    expect(document.body.textContent).not.toContain('Uploading…')
  })

  it('Reset of a staged upload restores the saved picture', async () => {
    handlers.upload_avatar_media = () => media('STAGED')
    const state = await open(saved({ user: { ...alex, avatar_hash: 'OLD' } }))
    await upload('Avatar')
    await vi.waitFor(() => {
      flushSync()
      expect(previewLine().querySelector('img')?.getAttribute('src')).toContain('STAGED')
    })
    click(button('Reset'))
    expect(previewLine().querySelector('img')!.getAttribute('src')).toContain('OLD')
    expect(state.dirty).toBe(false)
    expect(calls('update_profile')).toHaveLength(0)
  })

  it('more than 4 line breaks in the bio disables Save, blank leading lines not counted', async () => {
    await open()
    typeBio('\n\n\na\nb\nc\nd\ne')
    expect(document.body.textContent).not.toContain('Your bio can have at most 4 line breaks.')
    expect(button('Save').disabled).toBe(false)
    typeBio('a\nb\nc\nd\ne\nf')
    expect(document.querySelector('[role=alert]')!.textContent).toBe('Your bio can have at most 4 line breaks.')
    expect(button('Save').disabled).toBe(true)
  })

  it('pronouns count characters, not UTF-16 units, up to 40', async () => {
    await open()
    const box = document.querySelector<HTMLInputElement>('input[aria-label="Pronouns"]')!
    expect(box.maxLength).toBe(-1)
    type(box, '😀'.repeat(40))
    expect(document.body.textContent).not.toContain('Pronouns can be up to 40 characters.')
    expect(button('Save').disabled).toBe(false)
    type(box, '😀'.repeat(41))
    expect(document.querySelector('[role=alert]')!.textContent).toBe('Pronouns can be up to 40 characters.')
    expect(button('Save').disabled).toBe(true)
  })

  it('loads once you are known, and a failed load offers Retry', async () => {
    handlers.get_profile = () => Promise.reject('offline')
    app.state = emptyState()
    cmp = mount(ProfileSettings, { target: document.body, props: {} })
    flushSync()
    expect(document.body.textContent).toContain('Loading…')
    expect(calls('get_profile')).toHaveLength(0)
    app.state = { ...emptyState(), me: alex, people: { U1: { user: alex, online: true, last_seen_at: null } } } as never
    await vi.waitFor(() => {
      flushSync()
      expect(document.querySelector('[role=alert]')?.textContent).toContain('offline')
    })
    handlers.get_profile = () => saved()
    click(button('Retry'))
    await vi.waitFor(() => {
      flushSync()
      if (!document.querySelector('.preview .card')) throw new Error('not loaded')
    })
    expect(calls('get_profile')).toHaveLength(2)
  })

  it('radio groups: one tab stop each, arrow keys move and pick', async () => {
    await open()
    const radios = [...group('Card theme').querySelectorAll<HTMLButtonElement>('[role=radio]')]
    expect(radios.filter((r) => r.tabIndex === 0)).toEqual([option('Card theme', 'Plain')])
    const plain = radios.indexOf(option('Card theme', 'Plain'))
    radios[plain].focus()
    radios[plain].dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true, cancelable: true }))
    flushSync()
    const next = radios[(plain + 1) % radios.length]
    expect(document.activeElement).toBe(next)
    expect(next.getAttribute('aria-checked')).toBe('true')
    expect(next.tabIndex).toBe(0)
    expect(radios[plain].tabIndex).toBe(-1)
    // wraps backwards from the first
    const fonts = [...group('Name font').querySelectorAll<HTMLButtonElement>('[role=radio]')]
    fonts[0].focus()
    fonts[0].dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowLeft', bubbles: true, cancelable: true }))
    flushSync()
    expect(document.activeElement).toBe(fonts[fonts.length - 1])
    expect(fonts[fonts.length - 1].getAttribute('aria-checked')).toBe('true')
    const colours = [...group('Name colour').querySelectorAll<HTMLButtonElement>('[role=radio]')]
    expect(colours.filter((r) => r.tabIndex === 0)).toEqual([option('Name colour', 'Default')])
  })

  it('the footer sits flush with Settings\' padding through a shared variable', async () => {
    const { readFileSync } = await import('node:fs')
    const { resolve } = await import('node:path')
    const settings = readFileSync(resolve(__dirname, 'Settings.svelte'), 'utf8')
    const editor = readFileSync(resolve(__dirname, 'ProfileSettings.svelte'), 'utf8')
    expect(settings).toMatch(/\.content \{[^}]*--settings-pad-y: 28px;[^}]*padding: var\(--settings-pad-y\) 32px/)
    expect(editor).toMatch(/\.footer \{[^}]*bottom: calc\(-1 \* var\(--settings-pad-y, 28px\)\)/)
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
