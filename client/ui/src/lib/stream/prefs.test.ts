import { beforeEach, describe, expect, it, vi } from 'vitest'
import { loadDiagOverlay, loadGoLive, loadStreamVolumes, saveDiagOverlay, saveGoLive, saveStreamVolumes } from './prefs'

beforeEach(() => {
  const store = new Map<string, string>()
  vi.stubGlobal('localStorage', {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => void store.set(k, v),
  })
})

describe('prefs', () => {
  it('go_live_defaults_are_720p60_with_audio', () => {
    expect(loadGoLive()).toEqual({ res: 720, fps: 60, audio: true })
  })
  it('remembers the last Go Live choice', () => {
    saveGoLive({ res: 1080, fps: 30, audio: false })
    expect(loadGoLive()).toEqual({ res: 1080, fps: 30, audio: false })
  })
  it('falls back to defaults on corrupt storage', () => {
    localStorage.setItem('pulse.golive', '{not json')
    expect(loadGoLive()).toEqual({ res: 720, fps: 60, audio: true })
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
})
