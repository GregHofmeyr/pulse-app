// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fakeMatchMedia, FakeIntersectionObserver } from '../../lib/testMotion'

vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))

import { invoke } from '@tauri-apps/api/core'
import CardEditor from './CardEditor.svelte'
import { ProfileEditorState } from '../../lib/profileEditor.svelte'
import { app } from '../../lib/store.svelte'
import { emptyState } from '../../lib/state'
import { BIO_MAX, PRONOUNS_MAX } from '../../lib/profileDraft'
import type { Profile } from '../../lib/protocol/Profile'
import type { User } from '../../lib/protocol/User'

const alex: User = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
const profile = (over: { avatar?: string | null; banner?: string | null; bio?: string | null } = {}): Profile => ({
  user: { ...alex, avatar_hash: over.avatar ?? null, avatar_still: over.avatar ?? null },
  card: { banner_hash: over.banner ?? null, banner_still: over.banner ?? null, theme_top: '#3a3d46', theme_bottom: '#16171b', pronouns: 'they/them', bio: over.bio === undefined ? 'hello **there**' : over.bio },
  stats: { voice_seconds: 0, member_since: '2026-10-03T00:00:00Z', top_reaction: null, times_renamed: 0 },
  badges: [],
  name_archive: [],
  showcase: [],
  showcase_chosen: false,
})

let cmp: ReturnType<typeof mount> | undefined
let st: ProfileEditorState
let handlers: Record<string, (args: unknown, opts?: unknown) => unknown> = {}
const calls = (cmd: string) => vi.mocked(invoke).mock.calls.filter((c) => c[0] === cmd)

function show(p: Profile = profile()) {
  st = new ProfileEditorState(p)
  cmp = mount(CardEditor, { target: document.body, props: { state: st } })
  flushSync()
}
beforeEach(() => {
  handlers = {}
  Object.defineProperty(HTMLImageElement.prototype, 'naturalWidth', { configurable: true, get: () => 800 })
  Object.defineProperty(HTMLImageElement.prototype, 'naturalHeight', { configurable: true, get: () => 600 })
  vi.mocked(invoke).mockReset()
  vi.mocked(invoke).mockImplementation((async (cmd: string, args: unknown, opts: unknown) => handlers[cmd]?.(args, opts)) as never)
  app.state = { ...emptyState(), me: alex, people: { U1: { user: alex, online: true, last_seen_at: null } } } as never
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

const q = <T extends Element = HTMLElement>(sel: string) => document.querySelector<T>(sel)!
const click = (el: Element) => { (el as HTMLElement).click(); flushSync() }
const menu = () => document.querySelector<HTMLElement>('[role=menu]')
const items = () => [...(menu()?.querySelectorAll('[role=menuitem]') ?? [])].map((b) => b.textContent?.trim())
const type = (el: HTMLInputElement, v: string) => { el.value = v; el.dispatchEvent(new Event('input', { bubbles: true })); flushSync() }
const media = (hash: string) => ({ hash, still_hash: hash, width: 1, height: 1 })

/** Pick a file through the (hidden) input of an image menu, then Apply the crop. */
async function pick(kind: 'avatar' | 'banner') {
  const input = q<HTMLInputElement>(`input[type=file][data-kind=${kind}]`)
  Object.defineProperty(input, 'files', { configurable: true, value: [new File([new Uint8Array([1, 2, 3])], 'a.png', { type: 'image/png' })] })
  input.dispatchEvent(new Event('change', { bubbles: true }))
  flushSync()
  const img = await vi.waitFor(() => {
    const i = document.querySelector<HTMLImageElement>('[role=dialog][aria-label^="Crop"] img')
    if (!i) throw new Error('no crop image yet')
    return i
  })
  img.dispatchEvent(new Event('load'))
  flushSync()
  click([...q('[role=dialog][aria-label^="Crop"]').querySelectorAll('button')].find((b) => b.textContent?.trim() === 'Apply')!)
}

describe('the editable card', () => {
  it('pencils_open_change_remove_menu', () => {
    show()
    expect(menu()).toBeNull()
    // no picture yet: Change only
    click(q('button[aria-label="Edit banner"]'))
    expect(items()).toEqual(['Change'])
    expect(menu()!.hasAttribute('data-editor-layer')).toBe(true)
    click(q('button[aria-label="Edit banner"]'))
    expect(menu()).toBeNull()
    unmount(cmp!)
    document.body.innerHTML = ''
    show(profile({ avatar: 'AV', banner: 'BN' }))
    click(q('button[aria-label="Edit avatar"]'))
    expect(items()).toEqual(['Change', 'Remove'])
    // clicking the image itself opens the same menu
    click(q('button[aria-label="Edit avatar"]'))
    expect(menu()).toBeNull()
    click(q('.banner .hit'))
    expect(items()).toEqual(['Change', 'Remove'])
  })

  it('escape_closes_the_menu_and_is_marked_handled', () => {
    show()
    click(q('button[aria-label="Edit avatar"]'))
    const e = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true })
    window.dispatchEvent(e)
    flushSync()
    expect(e.defaultPrevented).toBe(true)
    expect(menu()).toBeNull()
  })

  it('change_opens_file_picker_then_crop', async () => {
    show()
    const picker = vi.spyOn(HTMLInputElement.prototype, 'click')
    click(q('button[aria-label="Edit banner"]'))
    click(document.querySelector('[role=menuitem]')!)
    expect(picker).toHaveBeenCalled()
    expect(menu()).toBeNull()
    expect(q<HTMLInputElement>('input[type=file][data-kind=banner]').accept).toBe('image/png,image/jpeg,image/gif,image/webp')
    picker.mockRestore()
    handlers.upload_banner_media = () => media('BN')
    await pick('banner')
    await vi.waitFor(() => { flushSync(); expect(st.draft.bannerHash).toBe('BN') })
    expect(calls('update_profile')).toHaveLength(0)
  })

  it('remove_clears_the_draft_hash', () => {
    show(profile({ avatar: 'AV' }))
    click(q('button[aria-label="Edit avatar"]'))
    click([...document.querySelectorAll('[role=menuitem]')].find((b) => b.textContent?.trim() === 'Remove')!)
    expect(st.draft.avatarHash).toBeNull()
    expect(menu()).toBeNull()
    expect(st.dirty).toBe(true)
  })

  it('upload_shows_spinner_on_the_image_and_error_under_the_card', async () => {
    let finish: ((m: unknown) => void) | undefined
    handlers.upload_avatar_media = () => new Promise((r) => (finish = r))
    show()
    expect(document.querySelector('.spinner')).toBeNull()
    await pick('avatar')
    await vi.waitFor(() => { flushSync(); expect(q('.ring .spinner')).not.toBeNull() })
    await vi.waitFor(() => expect(finish).toBeTypeOf('function'))
    finish!(media('AV'))
    await vi.waitFor(() => { flushSync(); expect(document.querySelector('.spinner')).toBeNull() })

    handlers.upload_banner_media = () => Promise.reject('that file is too big (8 MB at most)')
    await pick('banner')
    await vi.waitFor(() => { flushSync(); expect(q('[role=alert]').textContent).toBe('that file is too big (8 MB at most)') })
    // under the card, not inside it
    expect(q('.card').contains(q('[role=alert]'))).toBe(false)
    expect(q('.card').compareDocumentPosition(q('[role=alert]')) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
  })

  it('name_pronouns_bio_edit_in_place', () => {
    show()
    type(q<HTMLInputElement>('input[aria-label="Display name"]'), 'Alex A')
    type(q<HTMLInputElement>('input[aria-label="Pronouns"]'), 'he/him')
    expect(st.draft.displayName).toBe('Alex A')
    expect(st.draft.pronouns).toBe('he/him')
    click(q('.bio-view'))
    const box = q('[role=textbox][aria-label="Bio"]')
    box.textContent = 'new bio'
    box.dispatchEvent(new Event('input', { bubbles: true }))
    flushSync()
    expect(st.draft.bio).toBe('new bio')
    expect(st.dirty).toBe(true)
    expect(calls('update_profile')).toHaveLength(0)
  })

  it('name_problem_and_limits_show_under_the_field', () => {
    show()
    type(q<HTMLInputElement>('input[aria-label="Display name"]'), '   ')
    expect(q('.who [role=alert]').textContent).toBe('Names can be 1 to 32 characters.')
    type(q<HTMLInputElement>('input[aria-label="Display name"]'), 'alex')
    expect(document.querySelector('.who [role=alert]')).toBeNull()
    type(q<HTMLInputElement>('input[aria-label="Pronouns"]'), 'x'.repeat(PRONOUNS_MAX + 1))
    expect(q('.handle [role=alert]').textContent).toContain(`${PRONOUNS_MAX} characters`)
    click(q('.bio-view'))
    const box = q('[role=textbox][aria-label="Bio"]')
    box.textContent = 'a\n\n\n\n\nb'
    box.dispatchEvent(new Event('input', { bubbles: true }))
    flushSync()
    expect(q('.bio-field [role=alert]').textContent).toContain('4 line breaks')
    box.textContent = 'y'.repeat(BIO_MAX + 1)
    box.dispatchEvent(new Event('input', { bubbles: true }))
    flushSync()
    expect(q('.bio-field .counter').textContent).toBe(`${BIO_MAX + 1}/${BIO_MAX}`)
    expect(q('.bio-field .counter').classList.contains('over')).toBe(true)
  })

  it('bio_raw_while_focused_formatted_on_blur', () => {
    show()
    // blurred: markdown rendered, no raw box, no emoji button
    expect(q('.bio-view strong').textContent).toBe('there')
    expect(document.querySelector('[role=textbox]')).toBeNull()
    expect(document.querySelector('button[aria-label="Emoji"]')).toBeNull()
    // focus: the raw text and the emoji button
    click(q('.bio-view'))
    const box = q('[role=textbox][aria-label="Bio"]')
    expect(box.textContent).toBe('hello **there**')
    expect(document.querySelector('.bio-view')).toBeNull()
    expect(q('.bio-field button[aria-label="Emoji"]')).not.toBeNull()
    // leaving the field formats it again
    q('.bio-field').dispatchEvent(new FocusEvent('focusout', { bubbles: true, relatedTarget: document.body }))
    flushSync()
    expect(q('.bio-view strong').textContent).toBe('there')
    expect(document.querySelector('[role=textbox]')).toBeNull()
  })

  it('long_inputs_stay_inside_the_card', () => {
    show(profile({ bio: 'z'.repeat(190) }))
    const card = q('.card')
    const name = q<HTMLInputElement>('input[aria-label="Display name"]')
    type(name, 'W'.repeat(60))
    click(q('.bio-view'))
    const box = q('[role=textbox][aria-label="Bio"]')
    box.textContent = 'line\n'.repeat(8) + 'q'.repeat(190)
    box.dispatchEvent(new Event('input', { bubbles: true }))
    flushSync()
    for (const el of [name, box, q('.bio-field')]) expect(card.contains(el)).toBe(true)
    // jsdom does no layout: what keeps them in (ellipsis, the bio's fixed heights and scrolling) is in CardEditorCss.test.ts
    expect(st.draft.displayName).toHaveLength(60)
    expect(st.draft.bio.split('\n')).toHaveLength(9)
  })
})
