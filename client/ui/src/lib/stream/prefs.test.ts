import { beforeEach, describe, expect, it, vi } from 'vitest'
import { clampGoLive, loadDiagOverlay, loadGoLive, loadHidePreview, saveHidePreview, loadStreamVolumes, saveDiagOverlay, saveGoLive, saveStreamVolumes } from './prefs'

beforeEach(() => {
  const store = new Map<string, string>()
  vi.stubGlobal('localStorage', {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => void store.set(k, v),
  })
})

describe('prefs', () => {
  it('go_live_defaults_are_720p60_with_audio_and_ghosts_allowed', () => {
    expect(loadGoLive()).toEqual({ res: 720, fps: 60, audio: true, allowGhosts: true, sound: 'app' })
  })
  it('remembers the last Go Live choice', () => {
    saveGoLive({ res: 1080, fps: 30, audio: false, allowGhosts: false, sound: 'everything' })
    expect(loadGoLive()).toEqual({ res: 1080, fps: 30, audio: false, allowGhosts: false, sound: 'everything' })
  })
  it('a_choice_saved_before_ghosts_existed_allows_them', () => {
    localStorage.setItem('pulse.golive', JSON.stringify({ res: 1080, fps: 30, audio: false }))
    expect(loadGoLive().allowGhosts).toBe(true)
  })
  it('falls back to defaults on corrupt storage', () => {
    localStorage.setItem('pulse.golive', '{not json')
    expect(loadGoLive()).toEqual({ res: 720, fps: 60, audio: true, allowGhosts: true, sound: 'app' })
  })
  it('diag overlay defaults off and persists', () => {
    expect(loadDiagOverlay()).toBe(false)
    saveDiagOverlay(true)
    expect(loadDiagOverlay()).toBe(true)
  })
  it('stream volumes round-trip', () => {
    expect(loadStreamVolumes()).toEqual({})
    saveStreamVolumes({ u1: 80 })
    expect(loadStreamVolumes()).toEqual({ u1: 80 })
  })
  it('hide preview defaults off and persists', () => {
    expect(loadHidePreview()).toBe(false)
    saveHidePreview(true)
    expect(loadHidePreview()).toBe(true)
  })
})

describe('clampGoLive', () => {
  it('keeps a valid stored choice', () => {
    expect(clampGoLive({ res: 1080, fps: 30, audio: false, allowGhosts: false, sound: 'everything' }, 1440)).toEqual({ res: 1080, fps: 30, audio: false, allowGhosts: false, sound: 'everything' })
  })
  it('drops a resolution above the source to the largest offered', () => {
    expect(clampGoLive({ res: 1440, fps: 60, audio: true, allowGhosts: true, sound: 'app' }, 1080)).toEqual({ res: 1080, fps: 60, audio: true, allowGhosts: true, sound: 'app' })
  })
  it('a small source offers 720 only', () => {
    expect(clampGoLive({ res: 1080, fps: 60, audio: true, allowGhosts: true, sound: 'app' }, 480).res).toBe(720)
  })
  it('allow_ghosts_from_storage_must_be_a_boolean', () => {
    expect(clampGoLive({ res: 720, fps: 60, audio: true, allowGhosts: 'no' as never, sound: 'app' }, 1080).allowGhosts).toBe(true)
  })
  it('a_sound_source_from_storage_must_be_app_or_everything', () => {
    expect(clampGoLive({ res: 720, fps: 60, audio: true, allowGhosts: true, sound: 'all' as never }, 1080).sound).toBe('app')
    expect(clampGoLive({ res: 720, fps: 60, audio: true, allowGhosts: true, sound: 'everything' }, 1080).sound).toBe('everything')
  })
  it('a_choice_saved_before_the_sound_choice_is_this_app', () => {
    localStorage.setItem('pulse.golive', JSON.stringify({ res: 1080, fps: 30, audio: true, allowGhosts: true }))
    expect(loadGoLive().sound).toBe('app')
  })
  it('replaces junk stored values with the defaults', () => {
    const junk = { res: 999, fps: 24, audio: 'yes' } as unknown as Parameters<typeof clampGoLive>[0]
    expect(clampGoLive(junk, 1440)).toEqual({ res: 720, fps: 60, audio: true, allowGhosts: true, sound: 'app' })
  })
})

describe('call grid toggle', () => {
  it('show_everyone_defaults_on_and_is_remembered_per_user', async () => {
    const { loadShowEveryone, saveShowEveryone } = await import('./prefs')
    expect(loadShowEveryone('U1')).toBe(true)
    saveShowEveryone('U1', false)
    expect(loadShowEveryone('U1')).toBe(false)
    expect(loadShowEveryone('U2')).toBe(true)
    expect(localStorage.getItem('pulse.callgrid.everyone.U1')).not.toBeNull()
  })
})

describe('mini-preview corner', () => {
  it('defaults_bottom_right_and_is_remembered', async () => {
    const { loadMiniCorner, saveMiniCorner } = await import('./prefs')
    expect(loadMiniCorner()).toBe('br')
    saveMiniCorner('tl')
    expect(loadMiniCorner()).toBe('tl')
    expect(localStorage.getItem('pulse.stream.minicorner')).not.toBeNull()
  })
  it('junk_in_storage_falls_back_to_bottom_right', async () => {
    const { loadMiniCorner } = await import('./prefs')
    localStorage.setItem('pulse.stream.minicorner', JSON.stringify({ corner: 'middle' }))
    expect(loadMiniCorner()).toBe('br')
  })
})
