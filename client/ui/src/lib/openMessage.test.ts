import { beforeEach, describe, expect, it, vi } from 'vitest'

const jumpTo = vi.fn()
vi.mock('./jump', () => ({ jumpTo: (...a: unknown[]) => jumpTo(...a) }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(), convertFileSrc: (s: string) => s }))

import { openMessage } from './openMessage'
import { app, ui } from './store.svelte'
import { emptyState } from './state'
import type { Message } from './protocol/Message'

const msg = (o: Partial<Message>): Message => ({
  id: 'M1', channel_id: 'B1', author_id: 'U2', kind: 'normal', content: '', reply_to_id: null, created_at: '2026-10-01T00:00:00.000Z', edited_at: null,
  deleted: false, mentions: [], reactions: [], thread_root_id: null, quote: null, qoty_card: null, embeds: [], clip: null, ...o,
})

describe('openMessage in a builds channel', () => {
  beforeEach(() => {
    jumpTo.mockClear()
    ui.goTo = null
    app.state = { ...emptyState(), channels: { B1: { id: 'B1', server_id: 'S1', kind: 'builds', name: 'raven-builds', position: 0 } } }
  })

  it('a build or a comment on one opens its page, never a jump into a chat history', () => {
    expect(openMessage(msg({}))).toBe(true)
    expect(ui.goTo).toBe('B1')
    expect(ui.openBuild).toEqual({ channelId: 'B1', messageId: 'M1' })
    ui.goTo = null
    ui.openBuild = null
    expect(openMessage(msg({ id: 'R1', thread_root_id: 'M1' }))).toBe(true)
    expect(ui.goTo).toBe('B1')
    expect(ui.openBuild).toEqual({ channelId: 'B1', messageId: 'M1' })
    expect(ui.openReplies).toBeFalsy()
    expect(jumpTo).not.toHaveBeenCalled()
  })
})
