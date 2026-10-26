import { describe, expect, it } from 'vitest'
import { screenFor } from './screen'

describe('screenFor', () => {
  it('picks the screen from boot, sign-in and connection state', () => {
    expect(screenFor({ booting: true, signedIn: true, conn: 'connected' })).toBe('boot')
    expect(screenFor({ booting: false, signedIn: false, conn: 'connecting' })).toBe('login')
    expect(screenFor({ booting: false, signedIn: true, conn: 'reconnecting' })).toBe('shell')
    expect(screenFor({ booting: false, signedIn: true, conn: 'update_required' })).toBe('update')
  })
  it('a refused startup waits (through the retries) instead of showing login', () => {
    const v = { booting: false, signedIn: false, restoreRefused: true }
    expect(screenFor({ ...v, conn: 'too_many_devices' })).toBe('waiting')
    expect(screenFor({ ...v, conn: 'connecting' })).toBe('waiting')
    expect(screenFor({ ...v, conn: 'logged_out' })).toBe('login')
    expect(screenFor({ ...v, restoreRefused: false, conn: 'too_many_devices' })).toBe('login')
  })
})
