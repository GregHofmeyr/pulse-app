import { describe, expect, it } from 'vitest'
import { screenFor } from './screen'

describe('screenFor', () => {
  it('picks the screen from boot, sign-in and connection state', () => {
    expect(screenFor({ booting: true, signedIn: true, conn: 'connected' })).toBe('boot')
    expect(screenFor({ booting: false, signedIn: false, conn: 'connecting' })).toBe('login')
    expect(screenFor({ booting: false, signedIn: true, conn: 'reconnecting' })).toBe('shell')
    expect(screenFor({ booting: false, signedIn: true, conn: 'update_required' })).toBe('update')
  })
})
