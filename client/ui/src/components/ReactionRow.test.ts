// @vitest-environment jsdom
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Message } from '../lib/protocol/Message'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import ReactionRow from './ReactionRow.svelte'
import MessageItem from './MessageItem.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'

const msg = (over: Partial<Message> = {}): Message => ({
  id: 'M1', channel_id: 'C1', author_id: 'U2', kind: 'normal', content: 'hi',
  reply_to_id: null, created_at: '2026-10-01T00:00:00.000Z', edited_at: null, deleted: false, mentions: [],
  reactions: [{ emoji: '👍', count: 2, me: true }], ...over,
})
const load = (m: Message) => {
  app.state = { ...emptyState(), messages: { C1: [m] } }
}
const cur = () => app.state.messages['C1'][0]
const chips = () => [...document.querySelectorAll('.chip:not(.add)')] as HTMLButtonElement[]

let cmp: ReturnType<typeof mount> | undefined
const onError = vi.fn()
function show(m: Message) {
  load(m)
  cmp = mount(ReactionRow, { target: document.body, props: { message: app.state.messages['C1'][0], meId: 'U1', nameOf: (id: string | null) => id ?? '', onError } })
  flushSync()
}
beforeEach(() => {
  invoke.mockReset(); invoke.mockResolvedValue(undefined); onError.mockClear()
  Element.prototype.scrollIntoView = () => {}
})
const settle = () => new Promise((r) => setTimeout(r, 0))
function deferred<T = unknown>() {
  let resolve!: (v: T) => void
  let reject!: (e: unknown) => void
  const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej })
  return { promise, resolve, reject }
}
const calls = (cmd: string) => invoke.mock.calls.filter((c) => c[0] === cmd)
function showItem(m: Message) {
  load(m)
  cmp = mount(MessageItem, { target: document.body, props: {
    message: cur(), name: 'sam', nameOf: (id: string | null) => id ?? '', replyTo: null, grouped: false, mine: false,
    onReply: () => {}, onEdit: async () => {}, onDelete: () => {}, meId: 'U1', onError,
  } })
  flushSync()
}
const cell = (k: string) => [...document.querySelectorAll<HTMLButtonElement>('.cell')].find((c) => c.dataset.key === k)!
afterEach(() => { if (cmp) unmount(cmp); cmp = undefined; document.body.innerHTML = '' })

describe('ReactionRow', () => {
  it('clicking your highlighted chip removes your reaction', async () => {
    show(msg())
    chips()[0].click()
    await tick()
    expect(invoke).toHaveBeenCalledWith('remove_reaction', { messageId: 'M1', emoji: '👍' })
    expect(cur().reactions).toEqual([{ emoji: '👍', count: 1, me: false }])
  })

  it('a failed call restores the chip and reports the error', async () => {
    invoke.mockRejectedValueOnce('too many reactions')
    show(msg())
    chips()[0].click()
    await new Promise((r) => setTimeout(r, 0))
    expect(cur().reactions).toEqual([{ emoji: '👍', count: 2, me: true }])
    expect(onError).toHaveBeenCalledOnce()
  })

  it('deleted_custom_shows_name_chip', () => {
    const X1 = { id: 'X1', server_id: 'S1', name: 'party', hash: null, still_hash: null, animated: false, added_by: null, created_at: '2026-10-01T00:00:00.000Z' }
    show(msg({ reactions: [{ emoji: 'custom:X1', count: 3, me: false }, { emoji: 'custom:ZZ', count: 1, me: false }] }))
    app.state = { ...app.state, customEmoji: { X1 } } // X1 deleted (hash null); ZZ unknown
    flushSync()
    const gone = [...document.querySelectorAll('.gone')].map((e) => e.textContent)
    expect(gone).toEqual([':party:', ':custom:ZZ:'])
    expect(document.querySelector('.chip img')).toBeNull()
  })

  it('hover lists reactors, fetching fresh on each hover', async () => {
    invoke.mockResolvedValue(['U2', 'U3'])
    show(msg({ reactions: [{ emoji: '👍', count: 2, me: false }] }))
    const c = chips()[0]
    c.dispatchEvent(new Event('pointerenter'))
    await new Promise((r) => setTimeout(r, 0))
    flushSync()
    expect(document.querySelector('.tip')?.textContent).toBe('U2 and U3')
    c.dispatchEvent(new Event('pointerleave'))
    invoke.mockResolvedValue(['U2', 'U4']) // one left, one joined: same count
    c.dispatchEvent(new Event('pointerenter'))
    await new Promise((r) => setTimeout(r, 0))
    flushSync()
    expect(document.querySelector('.tip')?.textContent).toBe('U2 and U4')
    expect(invoke.mock.calls.filter((x) => x[0] === 'list_reactors')).toHaveLength(2)
  })

  it('failure after an event landed takes the server list, not a blind decrement', async () => {
    // Someone else's event (count 2, excludes me) arrives mid-flight, then add fails.
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'add_reaction') {
        app.state = { ...app.state, messages: { C1: [{ ...cur(), reactions: [{ emoji: '🎉', count: 4, me: true }] }] } }
        throw 'boom'
      }
      if (cmd === 'list_reactors') return ['U2', 'U3']
      return undefined
    })
    show(msg({ reactions: [{ emoji: '🎉', count: 2, me: false }] }))
    const { react } = await import('../lib/reactions')
    await react(() => app.state, (s) => (app.state = s), 'C1', 'M1', '🎉', true, 'U1', onError)
    expect(cur().reactions).toEqual([{ emoji: '🎉', count: 2, me: false }])
    expect(onError).toHaveBeenCalledOnce()
  })

  it('failure with no event and an unreachable reactor list undoes the flip', async () => {
    invoke.mockRejectedValue('offline')
    show(msg({ reactions: [{ emoji: '🎉', count: 2, me: false }] }))
    const { react } = await import('../lib/reactions')
    await react(() => app.state, (s) => (app.state = s), 'C1', 'M1', '🎉', true, 'U1', onError)
    expect(cur().reactions).toEqual([{ emoji: '🎉', count: 2, me: false }])
  })

  it('failure at the 100-reactor cap undoes the flip instead of trusting the list', async () => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_reactors') return Array.from({ length: 100 }, (_, i) => `X${i}`)
      if (cmd === 'add_reaction') throw 'boom'
      return undefined
    })
    show(msg({ reactions: [{ emoji: '🎉', count: 150, me: false }] }))
    const { react } = await import('../lib/reactions')
    await react(() => app.state, (s) => (app.state = s), 'C1', 'M1', '🎉', true, 'U1', onError)
    expect(cur().reactions).toEqual([{ emoji: '🎉', count: 150, me: false }])
  })

  it('an event landing during the settle fetch is not overwritten', async () => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'add_reaction') throw 'boom'
      if (cmd === 'list_reactors') {
        app.state = { ...app.state, messages: { C1: [{ ...cur(), reactions: [{ emoji: '🎉', count: 9, me: false }] }] } }
        return ['U2', 'U3'] // older snapshot
      }
      return undefined
    })
    show(msg({ reactions: [{ emoji: '🎉', count: 2, me: false }] }))
    const { react } = await import('../lib/reactions')
    await react(() => app.state, (s) => (app.state = s), 'C1', 'M1', '🎉', true, 'U1', onError)
    expect(cur().reactions).toEqual([{ emoji: '🎉', count: 9, me: false }])
  })

  it('failure after my own event landed keeps the committed reaction', async () => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'add_reaction') {
        app.state = { ...app.state, messages: { C1: [{ ...cur(), reactions: [{ emoji: '🎉', count: 3, me: true }] }] } }
        throw 'response lost'
      }
      if (cmd === 'list_reactors') return ['U1', 'U2', 'U3']
      return undefined
    })
    show(msg({ reactions: [{ emoji: '🎉', count: 2, me: false }] }))
    const { react } = await import('../lib/reactions')
    await react(() => app.state, (s) => (app.state = s), 'C1', 'M1', '🎉', true, 'U1', onError)
    expect(cur().reactions).toEqual([{ emoji: '🎉', count: 3, me: true }])
  })
})

describe('MessageItem reactions', () => {
  it('hidden_on_deleted_message', () => {
    load(msg({ deleted: true }))
    cmp = mount(MessageItem, { target: document.body, props: {
      message: cur(), name: 'sam', nameOf: () => 'sam', replyTo: null, grouped: false, mine: false,
      onReply: () => {}, onEdit: async () => {}, onDelete: () => {},
    } })
    flushSync()
    expect(document.querySelector('.reactions')).toBeNull()
    expect(document.querySelector('[aria-label="Add reaction"]')).toBeNull()
  })
})

describe('reaction toggles', () => {
  it('a second toggle while the first is in flight waits for it, then applies', async () => {
    const add = deferred()
    invoke.mockImplementation(async (cmd: string) => (cmd === 'add_reaction' ? add.promise : undefined))
    load(msg({ reactions: [{ emoji: '🎉', count: 1, me: false }] }))
    const { react } = await import('../lib/reactions')
    const r = (on: boolean) => react(() => app.state, (s) => (app.state = s), 'C1', 'M1', '🎉', on, 'U1', onError)
    const first = r(true) // a double click: add, then remove
    const second = r(false)
    await settle()
    expect(calls('remove_reaction')).toHaveLength(0)
    expect(cur().reactions).toEqual([{ emoji: '🎉', count: 2, me: true }])
    add.resolve(undefined)
    await Promise.all([first, second])
    expect(invoke.mock.calls.map((c) => c[0])).toEqual(['add_reaction', 'remove_reaction'])
    expect(cur().reactions).toEqual([{ emoji: '🎉', count: 1, me: false }])
  })

  it('picking an emoji you already hold is a no-op', async () => {
    showItem(msg({ reactions: [{ emoji: '👍', count: 2, me: true }] }))
    ;(document.querySelector('.actions [aria-label="Add reaction"]') as HTMLButtonElement).click()
    flushSync()
    cell('👍').click()
    await settle()
    expect(invoke).not.toHaveBeenCalled()
    expect(cur().reactions).toEqual([{ emoji: '👍', count: 2, me: true }])
  })

  it('the message smiley button opens the picker and adds the pick', async () => {
    showItem(msg({ reactions: [] }))
    ;(document.querySelector('.actions [aria-label="Add reaction"]') as HTMLButtonElement).click()
    flushSync()
    cell('🎉').click()
    flushSync()
    expect(document.querySelector('.cell')).toBeNull() // picker closed
    await settle()
    expect(invoke).toHaveBeenCalledWith('add_reaction', { messageId: 'M1', emoji: '🎉' })
    expect(cur().reactions).toEqual([{ emoji: '🎉', count: 1, me: true }])
  })

  it('a failed add removes the new chip again and reports the error', async () => {
    invoke.mockRejectedValue('nope')
    showItem(msg({ reactions: [] }))
    ;(document.querySelector('.actions [aria-label="Add reaction"]') as HTMLButtonElement).click()
    flushSync()
    cell('🎉').click()
    await settle()
    expect(cur().reactions).toEqual([])
    expect(onError).toHaveBeenCalledWith('nope')
  })

  it('both pickers open in the same fixed popover', () => {
    showItem(msg())
    ;(document.querySelector('.actions [aria-label="Add reaction"]') as HTMLButtonElement).click()
    flushSync()
    expect((document.querySelector('.emoji-popover') as HTMLElement).style.position).toBe('fixed')
    ;(document.querySelector('.actions [aria-label="Add reaction"]') as HTMLButtonElement).click()
    flushSync()
    expect(document.querySelector('.emoji-popover')).toBeNull()
    ;(document.querySelector('.reactions .add') as HTMLButtonElement).click()
    flushSync()
    expect((document.querySelector('.emoji-popover') as HTMLElement).style.position).toBe('fixed')
  })

  it('a fetch failure after my own event landed keeps the committed reaction', async () => {
    const { noteReactionEvent } = await import('../lib/reactions')
    const { applyEvent } = await import('../lib/state')
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'add_reaction') {
        const e = { t: 'ReactionAdded' as const, d: { channel_id: 'C1', message_id: 'M1', emoji: '🎉', user_id: 'U1', count: 3 } }
        noteReactionEvent(e, 'U1')
        app.state = applyEvent(app.state, e, 0)
        throw 'response lost'
      }
      throw 'offline'
    })
    show(msg({ reactions: [{ emoji: '🎉', count: 2, me: false }] }))
    const { react } = await import('../lib/reactions')
    await react(() => app.state, (s) => (app.state = s), 'C1', 'M1', '🎉', true, 'U1', onError)
    expect(cur().reactions).toEqual([{ emoji: '🎉', count: 3, me: true }])
    expect(onError).toHaveBeenCalledOnce()
  })

  it('a capped reactor list after my own event landed keeps the committed reaction', async () => {
    const { noteReactionEvent } = await import('../lib/reactions')
    const { applyEvent } = await import('../lib/state')
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'add_reaction') {
        const e = { t: 'ReactionAdded' as const, d: { channel_id: 'C1', message_id: 'M1', emoji: '🎉', user_id: 'U1', count: 151 } }
        noteReactionEvent(e, 'U1')
        app.state = applyEvent(app.state, e, 0)
        throw 'response lost'
      }
      if (cmd === 'list_reactors') return Array.from({ length: 100 }, (_, i) => `X${i}`)
    })
    show(msg({ reactions: [{ emoji: '🎉', count: 150, me: false }] }))
    const { react } = await import('../lib/reactions')
    await react(() => app.state, (s) => (app.state = s), 'C1', 'M1', '🎉', true, 'U1', onError)
    expect(cur().reactions).toEqual([{ emoji: '🎉', count: 151, me: true }])
  })
})

describe('reactor tooltip', () => {
  it('shows on keyboard focus, described by the chip, and hides on blur', async () => {
    invoke.mockResolvedValue(['U2', 'U3'])
    show(msg({ reactions: [{ emoji: '👍', count: 2, me: false }] }))
    const c = chips()[0]
    c.dispatchEvent(new FocusEvent('focus'))
    await settle()
    flushSync()
    const tip = document.querySelector('.tip')!
    expect(tip.textContent).toBe('U2 and U3')
    expect(tip.id).not.toBe('')
    expect(c.getAttribute('aria-describedby')).toBe(tip.id)
    c.dispatchEvent(new FocusEvent('blur'))
    flushSync()
    expect(document.querySelector('.tip')).toBeNull()
    expect(c.getAttribute('aria-describedby')).toBeNull()
  })

  it('a slow earlier fetch never overwrites a newer one', async () => {
    const slow = deferred<string[]>()
    invoke.mockReturnValueOnce(slow.promise).mockResolvedValueOnce(['U2', 'U4'])
    show(msg({ reactions: [{ emoji: '👍', count: 2, me: false }] }))
    const c = chips()[0]
    c.dispatchEvent(new Event('pointerenter'))
    c.dispatchEvent(new Event('pointerleave'))
    c.dispatchEvent(new Event('pointerenter'))
    await settle()
    flushSync()
    expect(document.querySelector('.tip')?.textContent).toBe('U2 and U4')
    slow.resolve(['U2', 'U3'])
    await settle()
    flushSync()
    expect(document.querySelector('.tip')?.textContent).toBe('U2 and U4')
  })
})
