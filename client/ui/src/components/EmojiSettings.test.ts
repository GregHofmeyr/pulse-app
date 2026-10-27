// @vitest-environment jsdom
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
// ui_log apart, so a logged failure doesn't use up the next queued reply
const uiLog = vi.fn((_args: unknown) => {})
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, args?: never) => (cmd === 'ui_log' ? Promise.resolve(uiLog(args)) : invoke(cmd, args)),
}))

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
const calls = (cmd: string) => invoke.mock.calls.filter((c) => c[0] === cmd)
const text = (t: string) => [...document.querySelectorAll('button')].find((b) => b.textContent === t)!

async function upload(name = 'Big Pog!.gif') {
  const input = document.querySelector<HTMLInputElement>('input[type=file]')!
  const file = new File([new Uint8Array([1, 2, 3])], name)
  Object.defineProperty(input, 'files', { value: [file], configurable: true })
  input.dispatchEvent(new Event('change', { bubbles: true }))
  await settle()
  flushSync()
}

beforeEach(() => { invoke.mockReset(); invoke.mockResolvedValue(undefined); uiLog.mockClear() })
afterEach(() => { if (cmp) unmount(cmp); cmp = undefined; document.body.innerHTML = '' })

describe('EmojiSettings', () => {
  it('default_name_from_file', () => {
    expect(defaultName('Big Pog!.gif')).toBe('Big_Pog')
    expect(defaultName('a.png')).toBe('a_')
    expect(defaultName('x'.repeat(50) + '.png')).toHaveLength(32)
    expect(defaultName('..gif')).toBe('__')
    expect(defaultName('café.png')).toBe('caf')
    expect(defaultName('日本.png')).toBe('__')
    expect(defaultName('a.b.c.png')).toBe('a_b_c')
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

  const nameInput = () => document.querySelector<HTMLInputElement>('input[aria-label="Emoji name"]')!
  const typeName = (v: string) => {
    const i = nameInput()
    i.value = v
    i.dispatchEvent(new Event('input', { bubbles: true }))
    flushSync()
  }

  it('Cancel and Esc on the add form clear a shown error', async () => {
    for (const close of [() => text('Cancel').click(), () => nameInput().dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))]) {
      invoke.mockReset()
      invoke.mockResolvedValueOnce(info).mockRejectedValueOnce('that name is already taken')
      show()
      await upload()
      text('Save').click()
      await settle()
      flushSync()
      expect(document.querySelector('[role=alert]')).not.toBeNull()
      close()
      flushSync()
      expect(document.querySelector('input[aria-label="Emoji name"]')).toBeNull()
      expect(document.querySelector('[role=alert]')).toBeNull()
      unmount(cmp!); cmp = undefined; document.body.innerHTML = ''
    }
  })

  it('Esc inside the add form is marked handled, so a surrounding dialog stays open', async () => {
    invoke.mockResolvedValueOnce(info)
    show()
    await upload()
    const e = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true })
    nameInput().dispatchEvent(e)
    expect(e.defaultPrevented).toBe(true)
  })

  const uiLogs = () => uiLog.mock.calls.map((c) => (c[0] as { line: string }).line)

  const hint = () => document.querySelector('.name-hint')!.textContent!.trim()

  it('an invalid name says why while typing, on Save and on Enter, logs it, and sends nothing', async () => {
    invoke.mockResolvedValueOnce(info)
    show()
    await upload()
    const input = nameInput()
    expect(input.getAttribute('aria-describedby')).toBe(document.querySelector('.name-hint')!.id)
    expect(input.getAttribute('aria-invalid')).toBe('false')
    for (const bad of ['', 'a', 'bad name', 'cat-loaf', 'x'.repeat(33), 'caf\u00e9']) {
      typeName(bad)
      expect(hint(), JSON.stringify(bad)).toMatch(/letters, numbers or underscores/)
      expect(input.getAttribute('aria-invalid')).toBe('true')
    }
    typeName('cat loaf')
    text('Save').click()
    await settle()
    flushSync()
    expect(document.querySelector('[role=alert]')!.textContent).toMatch(/no spaces or dashes/)
    expect(uiLog).toHaveBeenCalledTimes(1)
    // typing again clears the refused-name alert (the hint keeps saying it while it's still wrong)
    typeName('cat loaf2')
    expect(document.querySelector('[role=alert]')).toBeNull()
    // Enter on its own says why too
    nameInput().dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    await settle()
    flushSync()
    expect(document.querySelector('[role=alert]')!.textContent).toMatch(/no spaces or dashes/)
    expect(uiLog).toHaveBeenCalledTimes(2)
    expect(calls('create_emoji')).toHaveLength(0)
    expect(uiLogs().every((l) => l.includes('emoji: name refused'))).toBe(true)
    // the log says why, never what was typed or the file's name
    expect(uiLogs().every((l) => !l.includes('cat loaf') && !l.includes('Big Pog'))).toBe(true)
    typeName(' ok_1 ')
    expect(hint()).toBe('')
    expect(input.getAttribute('aria-invalid')).toBe('false')
    text('Save').click()
    await settle()
    expect(calls('create_emoji')).toHaveLength(1)
  })

  it('a preview that will not load says so and logs it', async () => {
    invoke.mockResolvedValueOnce(info)
    show()
    await upload()
    document.querySelector('img')!.dispatchEvent(new Event('error'))
    flushSync()
    expect(document.querySelector('[role=alert]')!.textContent).toMatch(/preview couldn.t be shown/)
    expect(uiLogs().some((l) => l.includes('emoji: preview failed'))).toBe(true)
  })

  it('a failed upload says why, logs it, and the next uploads still work', async () => {
    invoke.mockRejectedValueOnce('Only PNG, JPEG, GIF or WebP images')
    show()
    await upload('cat.webp')
    expect(document.querySelector('[role=alert]')!.textContent).toBe('Only PNG, JPEG, GIF or WebP images')
    expect(uiLogs().some((l) => l.includes('emoji: upload failed'))).toBe(true)
    // second: uploads, but the save is refused by the server
    invoke.mockResolvedValueOnce(info).mockRejectedValueOnce('this server already has an emoji with that name')
    await upload('cat2.webp')
    expect(document.querySelector('[role=alert]')).toBeNull() // a new attempt clears the old error
    text('Save').click()
    await settle()
    flushSync()
    expect(document.querySelector('[role=alert]')!.textContent).toBe('this server already has an emoji with that name')
    expect(uiLogs().some((l) => l.includes('emoji: save failed'))).toBe(true)
    text('Cancel').click()
    flushSync()
    // third: works end to end
    invoke.mockResolvedValueOnce({ ...info, hash: 'third' }).mockResolvedValueOnce(emoji('X9', 'cat3'))
    await upload('cat3.webp')
    text('Save').click()
    await settle()
    flushSync()
    expect(calls('create_emoji').at(-1)![1]).toEqual({ serverId: 'S1', name: 'cat3', mediaHash: 'third' })
    expect(document.querySelector('[role=alert]')).toBeNull()
    const file = document.querySelector<HTMLInputElement>('input[type=file]')!
    expect(file.disabled).toBe(false)
    expect(uiLogs().every((l) => !/cat\d?\.webp/.test(l))).toBe(true)
  })

  it('a rename shows the name rule while it is wrong', async () => {
    show([emoji('X1', 'pog')])
    btn('Rename :pog:').click()
    await tick()
    const input = document.querySelector<HTMLInputElement>('input[aria-label="Rename emoji"]')!
    const rhint = () => document.getElementById(input.getAttribute('aria-describedby')!)!.textContent!.trim()
    expect(rhint()).toBe('')
    input.value = 'pog dog'
    input.dispatchEvent(new Event('input', { bubbles: true }))
    flushSync()
    expect(rhint()).toMatch(/letters, numbers or underscores/)
    expect(input.getAttribute('aria-invalid')).toBe('true')
  })

  it('at the cap, says why adding is off', () => {
    show(Array.from({ length: 100 }, (_, i) => emoji('X' + i, 'e' + i)))
    expect(document.querySelector<HTMLInputElement>('input[type=file]')!.disabled).toBe(true)
    expect(document.body.textContent).toMatch(/100 emoji, the most it can hold; delete one first/)
  })

  it('a double-clicked delete sends one request, and Cancel clears an error', async () => {
    show([emoji('X1', 'pog')])
    btn('Delete :pog:').click()
    await tick()
    let done!: () => void
    invoke.mockImplementationOnce(() => new Promise<void>((r) => { done = r }))
    text('Delete').click()
    text('Delete').click()
    done()
    await settle()
    expect(calls('delete_emoji')).toHaveLength(1)
    invoke.mockRejectedValueOnce('slow down a little, then try again')
    btn('Delete :pog:').click()
    await tick()
    text('Delete').click()
    await settle()
    flushSync()
    expect(document.querySelector('[role=alert]')).not.toBeNull()
    text('Cancel').click()
    flushSync()
    expect(document.querySelector('[role=alert]')).toBeNull()
  })

  it('rename and delete failures, and an invalid rename, say why and are logged', async () => {
    show([emoji('X1', 'pog')])
    btn('Rename :pog:').click()
    await tick()
    const input = () => document.querySelector<HTMLInputElement>('input[aria-label="Rename emoji"]')!
    input().value = 'pog dog'
    input().dispatchEvent(new Event('input', { bubbles: true }))
    input().dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    await settle()
    flushSync()
    expect(document.querySelector('[role=alert]')!.textContent).toMatch(/no spaces or dashes/)
    expect(calls('rename_emoji')).toHaveLength(0)
    invoke.mockRejectedValueOnce('slow down a little, then try again')
    input().value = 'pogdog'
    input().dispatchEvent(new Event('input', { bubbles: true }))
    input().dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    await settle()
    flushSync()
    expect(document.querySelector('[role=alert]')!.textContent).toBe('slow down a little, then try again')
    input().dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    flushSync()
    invoke.mockRejectedValueOnce("can't reach the server: couldn't connect")
    btn('Delete :pog:').click()
    await tick()
    text('Delete').click()
    await settle()
    flushSync()
    expect(document.querySelector('[role=alert]')!.textContent).toBe("can't reach the server: couldn't connect")
    const logs = uiLogs()
    for (const what of ['emoji: name refused', 'emoji: rename failed', 'emoji: delete failed']) {
      expect(logs.some((l) => l.includes(what)), what).toBe(true)
    }
  })

  it('enter twice on a rename renames once', async () => {
    show([emoji('X1', 'pog')])
    btn('Rename :pog:').click()
    await tick()
    let done!: () => void
    invoke.mockImplementationOnce(() => new Promise<void>((r) => { done = r }))
    const input = document.querySelector<HTMLInputElement>('input[aria-label="Rename emoji"]')!
    const enter = () => input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    enter()
    enter()
    done()
    await settle()
    expect(calls('rename_emoji')).toHaveLength(1)
  })

  it('Esc on a rename clears its error, and an invalid rename is not sent', async () => {
    invoke.mockRejectedValueOnce('that name is already taken')
    show([emoji('X1', 'pog')])
    btn('Rename :pog:').click()
    await tick()
    const input = () => document.querySelector<HTMLInputElement>('input[aria-label="Rename emoji"]')!
    const press = (k: string) => input().dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true }))
    input().value = 'bad name'
    input().dispatchEvent(new Event('input', { bubbles: true }))
    press('Enter')
    await settle()
    expect(calls('rename_emoji')).toHaveLength(0)
    input().value = 'taken'
    input().dispatchEvent(new Event('input', { bubbles: true }))
    press('Enter')
    await settle()
    flushSync()
    expect(document.querySelector('[role=alert]')).not.toBeNull()
    press('Escape')
    flushSync()
    expect(document.querySelector('[role=alert]')).toBeNull()
  })

  it('a rename in flight does not block saving a new emoji', async () => {
    show([emoji('X1', 'pog')])
    invoke.mockResolvedValueOnce(info)
    await upload()
    btn('Rename :pog:').click()
    await tick()
    invoke.mockImplementationOnce(() => new Promise<void>(() => {}))
    const input = document.querySelector<HTMLInputElement>('input[aria-label="Rename emoji"]')!
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }))
    flushSync()
    expect((text('Save') as HTMLButtonElement).disabled).toBe(false)
  })
})
