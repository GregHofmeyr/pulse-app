// @vitest-environment jsdom
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import EmojiSettings from './EmojiSettings.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import { defaultName } from '../lib/emoji'
import type { CustomEmoji } from '../lib/protocol/CustomEmoji'

const server = { id: 'S1', name: 'Home', icon_hash: null }
const emoji = (id: string, name: string): CustomEmoji => ({
  id, server_id: 'S1', name, hash: 'h' + id, still_hash: null, animated: false, added_by: 'U2', created_at: '2026-10-01T00:00:00.000Z',
})
const info = { hash: 'newhash', content_type: 'image/webp', width: 128, height: 128, frames: 1, animated: false, still_hash: null }

let cmp: ReturnType<typeof mount> | undefined
const show = (list: CustomEmoji[] = []) => {
  app.state = { ...emptyState(), customEmoji: Object.fromEntries(list.map((e) => [e.id, e])),
    members: { S1: [{ user: { id: 'U2', username: 'sam', avatar_hash: null }, nickname: null }] } }
  cmp = mount(EmojiSettings, { target: document.body, props: { server } })
  flushSync()
}
const settle = () => new Promise((r) => setTimeout(r, 0))
const btn = (label: string) => document.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)!
const text = (t: string) => [...document.querySelectorAll('button')].find((b) => b.textContent === t)!

async function upload(name = 'Big Pog!.gif') {
  const input = document.querySelector<HTMLInputElement>('input[type=file]')!
  const file = new File([new Uint8Array([1, 2, 3])], name)
  Object.defineProperty(input, 'files', { value: [file], configurable: true })
  input.dispatchEvent(new Event('change', { bubbles: true }))
  await settle()
  flushSync()
}

beforeEach(() => { invoke.mockReset(); invoke.mockResolvedValue(undefined) })
afterEach(() => { if (cmp) unmount(cmp); cmp = undefined; document.body.innerHTML = '' })

describe('EmojiSettings', () => {
  it('default_name_from_file', () => {
    expect(defaultName('Big Pog!.gif')).toBe('Big_Pog')
    expect(defaultName('a.png')).toBe('a_')
    expect(defaultName('x'.repeat(50) + '.png')).toHaveLength(32)
  })

  it('lists live emoji of this server with count and adder', () => {
    show([emoji('X1', 'pog'), { ...emoji('X2', 'gone'), hash: null }, { ...emoji('X3', 'other'), server_id: 'S2' }])
    expect(document.body.textContent).toContain(':pog:')
    expect(document.body.textContent).not.toContain(':gone:')
    expect(document.body.textContent).not.toContain(':other:')
    expect(document.body.textContent).toContain('1 / 100')
    expect(document.body.textContent).toContain('sam')
  })

  it('upload_then_save_calls_create_with_hash', async () => {
    invoke.mockResolvedValueOnce(info)
    show()
    await upload()
    expect(invoke).toHaveBeenCalledWith('upload_emoji_media', expect.anything())
    expect(document.querySelectorAll('img').length).toBe(2)
    expect(document.querySelector<HTMLInputElement>('input[aria-label="Emoji name"]')!.value).toBe('Big_Pog')
    text('Save').click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('create_emoji', { serverId: 'S1', name: 'Big_Pog', mediaHash: 'newhash' })
  })

  it('enter_twice_saves_once', async () => {
    invoke.mockResolvedValueOnce(info)
    show()
    await upload()
    let done!: () => void
    invoke.mockImplementationOnce(() => new Promise<void>((r) => { done = r }))
    const name = document.querySelector<HTMLInputElement>('input[aria-label="Emoji name"]')!
    const enter = () => name.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    enter()
    enter()
    done()
    await settle()
    expect(invoke.mock.calls.filter(([cmd]) => cmd === 'create_emoji')).toHaveLength(1)
  })

  it('delete_asks_first', async () => {
    show([emoji('X1', 'pog')])
    btn('Delete :pog:').click()
    await tick()
    expect(invoke).not.toHaveBeenCalled()
    text('Delete').click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('delete_emoji', { id: 'X1' })
  })

  it('rename_saves_on_enter_and_cancels_on_escape', async () => {
    show([emoji('X1', 'pog')])
    btn('Rename :pog:').click()
    await tick()
    const input = document.querySelector<HTMLInputElement>('input[aria-label="Rename emoji"]')!
    input.value = 'pogger'
    input.dispatchEvent(new Event('input', { bubbles: true }))
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    await settle()
    expect(invoke).toHaveBeenCalledWith('rename_emoji', { id: 'X1', name: 'pogger' })

    btn('Rename :pog:').click()
    await tick()
    document.querySelector('input[aria-label="Rename emoji"]')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    await tick()
    expect(document.querySelector('input[aria-label="Rename emoji"]')).toBeNull()
    expect(invoke).toHaveBeenCalledTimes(1)
  })

  it('server_error_shown_inline', async () => {
    invoke.mockResolvedValueOnce(info).mockRejectedValueOnce('that name is already taken')
    show()
    await upload()
    text('Save').click()
    await settle()
    flushSync()
    expect(document.querySelector('[role=alert]')!.textContent).toBe('that name is already taken')
    expect(document.querySelector('input[aria-label="Emoji name"]')).not.toBeNull() // stays open to fix the name
  })
})
