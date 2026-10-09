import { describe, expect, it } from 'vitest'
import { defaultServer } from './config'

describe('defaultServer', () => {
  it('uses the build-time address, else the local dev server', () => {
    expect(defaultServer({ VITE_DEFAULT_SERVER: 'https://pulsechat.co.za' })).toBe('https://pulsechat.co.za')
    expect(defaultServer({ VITE_DEFAULT_SERVER: '  ' })).toBe('http://127.0.0.1:7890')
    expect(defaultServer({})).toBe('http://127.0.0.1:7890')
  })
})
