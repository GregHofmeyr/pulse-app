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

  it('updateInvite names its arguments (#80)', async () => {
    await api.updateInvite('abcDEF2345', { join_text: 'Hop in' })
    expect(invoke).toHaveBeenCalledWith('update_invite', { code: 'abcDEF2345', request: { join_text: 'Hop in' } })
  })

  it('setServerIcon and renameServer name their arguments (#55)', async () => {
    await api.setServerIcon('S1', 'ab')
    expect(invoke).toHaveBeenCalledWith('set_server_icon', { serverId: 'S1', hash: 'ab' })
    await api.setServerIcon('S1', null)
    expect(invoke).toHaveBeenCalledWith('set_server_icon', { serverId: 'S1', hash: null })
    await api.renameServer('S1', 'Lounge')
    expect(invoke).toHaveBeenCalledWith('rename_server', { serverId: 'S1', name: 'Lounge' })
  })

  it('setServerStatus names its arguments (#154)', async () => {
    await api.setServerStatus('S1', 'dnd')
    expect(invoke).toHaveBeenCalledWith('set_server_status', { serverId: 'S1', status: 'dnd' })
    await api.setServerStatus('S1', null)
    expect(invoke).toHaveBeenCalledWith('set_server_status', { serverId: 'S1', status: null })
  })

  it('setBlock names its arguments (#474)', async () => {
    await api.setBlock('U2', 'hide')
    expect(invoke).toHaveBeenCalledWith('set_block', { userId: 'U2', mode: 'hide' })
    await api.setBlock('U2', null)
    expect(invoke).toHaveBeenCalledWith('set_block', { userId: 'U2', mode: null })
  })

  it('setServerBanner names its arguments (#475)', async () => {
    await api.setServerBanner('S1', 'ab')
    expect(invoke).toHaveBeenCalledWith('set_server_banner', { serverId: 'S1', hash: 'ab' })
    await api.setServerBanner('S1', null)
    expect(invoke).toHaveBeenCalledWith('set_server_banner', { serverId: 'S1', hash: null })
  })

  it('getProfile and updateProfile name their arguments', async () => {
    await api.getProfile('U1')
    expect(invoke).toHaveBeenCalledWith('get_profile', { userId: 'U1' })
    const request = {
      avatar_hash: null, banner_hash: null, theme_top: '#3a3d46', theme_bottom: '#16171b',
      style: { font: null, colors: [] }, pronouns: null, bio: null, display_name: 'alex',
    }
    await api.updateProfile(request)
    expect(invoke).toHaveBeenCalledWith('update_profile', { request })
  })
})

describe('builds', () => {
  it('listBuilds sends every filter, absent ones as null', async () => {
    await api.listBuilds('C1', { game: 'G1', archived: true, q: 'raven' }, 'M9')
    await api.listBuilds('C1')
    expect(invoke.mock.calls).toEqual([
      ['list_builds', { channelId: 'C1', game: 'G1', archived: true, q: 'raven', before: 'M9' }],
      ['list_builds', { channelId: 'C1', game: null, archived: false, q: null, before: null }],
    ])
  })

  it('archiveManyBuilds and deleteManyBuilds send the ids to the batch commands', async () => {
    await api.archiveManyBuilds('C1', ['M1', 'M2'], true)
    await api.deleteManyBuilds('C1', ['M3'])
    expect(invoke.mock.calls).toEqual([
      ['archive_many_builds', { channelId: 'C1', body: { message_ids: ['M1', 'M2'], archived: true } }],
      ['delete_many_builds', { channelId: 'C1', body: { message_ids: ['M3'] } }],
    ])
  })

  it('setBuildFavourite and favouriteManyBuilds star builds for you', async () => {
    await api.setBuildFavourite('M1', true)
    await api.favouriteManyBuilds('C1', ['M1', 'M2'], false)
    expect(invoke.mock.calls).toEqual([
      ['set_build_favourite', { messageId: 'M1', favourite: true }],
      ['favourite_many_builds', { channelId: 'C1', body: { message_ids: ['M1', 'M2'], favourite: false } }],
    ])
  })
})
