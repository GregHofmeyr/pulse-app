import { describe, it, expect, vi, beforeEach } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import { api } from './tauri'

beforeEach(() => invoke.mockClear())

describe('clip jobs', () => {
  it('sendClip maps the pieces to source milliseconds for the core', async () => {
    await api.sendClip({
      source: 's1',
      channelId: 'C1',
      spans: [{ start: 2000, end: 9000 }, { start: 20000, end: 25500 }],
      preset: 'light',
      caption: 'ace',
      gameId: null,
    })
    expect(invoke).toHaveBeenCalledWith('send_clip', {
      request: {
        source: 's1',
        channel_id: 'C1',
        spans: [{ start_ms: 2000, end_ms: 9000 }, { start_ms: 20000, end_ms: 25500 }],
        preset: 'light',
        caption: 'ace',
        game_id: null,
      },
    })
  })

  it('retry, cancel and list name their job', async () => {
    await api.retryClip('j1')
    await api.cancelClip('j1')
    await api.clipJobs()
    expect(invoke.mock.calls).toEqual([
      ['retry_clip', { id: 'j1' }],
      ['cancel_clip', { id: 'j1' }],
      ['clip_jobs', undefined],
    ])
  })
})
