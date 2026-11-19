// `pulse://invite/<code>` links (#476) and pasted invite links (#80).
import { afterEach, describe, expect, it, vi } from 'vitest'

const handlers: Record<string, (e: { payload: unknown }) => void> = {}
vi.mock('@tauri-apps/api/event', () => ({
  listen: async (name: string, h: (e: { payload: unknown }) => void) => {
    handlers[name] = h
    return () => delete handlers[name]
  },
}))
const invoke = vi.fn(async (_cmd: string): Promise<unknown> => null)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string) => invoke(cmd) }))

import { inviteLinks, splitInviteLink, startInviteLinks } from './inviteLinks.svelte'

afterEach(() => {
  inviteLinks.code = null
  invoke.mockReset()
})

describe('invite links', () => {
  it('takes the link that started the app, then each one that arrives; anything malformed is ignored', async () => {
    invoke.mockResolvedValue('abcDEF2345')
    const off = await startInviteLinks()
    expect(invoke).toHaveBeenCalledWith('take_invite_link')
    expect(inviteLinks.code).toBe('abcDEF2345')
    handlers['invite-link']({ payload: 'ghjkmnPQRS' })
    expect(inviteLinks.code).toBe('ghjkmnPQRS')
    handlers['invite-link']({ payload: '../settings' })
    expect(inviteLinks.code).toBe('ghjkmnPQRS')
    off()
    expect(handlers['invite-link']).toBeUndefined()
  })

  it('splits a pasted link into its server and code; anything else is the code as typed', () => {
    expect(splitInviteLink(' https://pulse.example/i/abcDEF2345 ')).toEqual({ server: 'https://pulse.example', code: 'abcDEF2345' })
    expect(splitInviteLink('http://localhost:7890/i/abcDEF2345/')).toEqual({ server: 'http://localhost:7890', code: 'abcDEF2345' })
    expect(splitInviteLink('abcDEF2345')).toEqual({ server: null, code: 'abcDEF2345' })
    expect(splitInviteLink('pulse://invite/abcDEF2345')).toEqual({ server: null, code: 'pulse://invite/abcDEF2345' })
    expect(splitInviteLink('https://pulse.example/elsewhere')).toEqual({ server: null, code: 'https://pulse.example/elsewhere' })
  })
})
