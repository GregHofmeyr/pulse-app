import { describe, it, expect, vi, beforeEach } from 'vitest'

const invoke = vi.fn(async (_cmd: string, _args?: unknown, _options?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never, options?: never) => (options === undefined ? invoke(cmd, args) : invoke(cmd, args, options)) }))

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

describe('clip player', () => {
  it('setFullscreen sends the on flag to the core', async () => {
    await api.setFullscreen(true)
    expect(invoke).toHaveBeenCalledWith('set_fullscreen', { on: true })
  })
})

describe('profile pictures', () => {
  it('uploadAvatar sends the crop header', async () => {
    const bytes = new Uint8Array([1, 2, 3])
    await api.uploadAvatar(bytes, { x: 1, y: 2, w: 3, h: 4 })
    expect(invoke).toHaveBeenCalledWith('upload_avatar_media', bytes, { headers: { 'x-pulse-crop': '1,2,3,4' } })
  })

  it('uploadAvatar without a crop sends no header', async () => {
    const bytes = new Uint8Array([1, 2, 3])
    await api.uploadAvatar(bytes, null)
    expect(invoke).toHaveBeenCalledWith('upload_avatar_media', bytes)
  })

  it('uploadBanner uses its own command', async () => {
    const bytes = new Uint8Array([9])
    await api.uploadBanner(bytes, { x: 0, y: 0, w: 5, h: 2 })
    expect(invoke).toHaveBeenCalledWith('upload_banner_media', bytes, { headers: { 'x-pulse-crop': '0,0,5,2' } })
  })

  it('getProfile and updateProfile name their arguments', async () => {
    await api.getProfile('U1')
    expect(invoke).toHaveBeenCalledWith('get_profile', { userId: 'U1' })
  })
})
