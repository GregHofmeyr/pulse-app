// Server invites (#80), the pure side: invite links made and found, the dialog's choices, and what a card says.
import { describe, expect, it } from 'vitest'
import { cardView, countsLine, expiresText, expiryChoices, inviteCodesIn, inviteLink, stateFromError, usesChoices, usesText, wellFormedCode } from './serverInvites'
import { applyEvent, emptyState } from './state'

const ORIGIN = 'https://pulsechat.co.za'

describe('invite links', () => {
  it('are made from the signed-in server', () => {
    expect(inviteLink(ORIGIN, 'abcDEF2345')).toBe('https://pulsechat.co.za/i/abcDEF2345')
    expect(inviteLink('http://localhost:8080/', 'abcDEF2345')).toBe('http://localhost:8080/i/abcDEF2345')
  })

  it('are found in a message: the server host only, a whole code only, each once, at most three', () => {
    expect(inviteCodesIn('join us https://pulsechat.co.za/i/abcDEF2345 !', ORIGIN)).toEqual(['abcDEF2345'])
    expect(inviteCodesIn('<https://PulseChat.co.za/i/abcDEF2345>', ORIGIN)).toEqual(['abcDEF2345'])
    expect(inviteCodesIn('https://www.pulsechat.co.za/i/abcDEF2345?x=1', ORIGIN)).toEqual(['abcDEF2345'])
    // another site, a code too long or short, a look-alike character (0, O, 1, l, I)
    expect(inviteCodesIn('https://evil.example/i/abcDEF2345', ORIGIN)).toEqual([])
    expect(inviteCodesIn('https://pulsechat.co.za.evil.example/i/abcDEF2345', ORIGIN)).toEqual([])
    expect(inviteCodesIn('https://pulsechat.co.za/i/abcDEF23456', ORIGIN)).toEqual([])
    expect(inviteCodesIn('https://pulsechat.co.za/i/abcDEF234', ORIGIN)).toEqual([])
    expect(inviteCodesIn('https://pulsechat.co.za/i/abcDEF2340', ORIGIN)).toEqual([])
    expect(inviteCodesIn('https://pulsechat.co.za/i/abcDEFl345', ORIGIN)).toEqual([])
    const many = ['aaaaaaaaaa', 'bbbbbbbbbb', 'aaaaaaaaaa', 'cccccccccc', 'dddddddddd'].map((c) => `${ORIGIN}/i/${c}`).join(' ')
    expect(inviteCodesIn(many, ORIGIN)).toEqual(['aaaaaaaaaa', 'bbbbbbbbbb', 'cccccccccc'])
    // not signed in yet: nothing
    expect(inviteCodesIn(`${ORIGIN}/i/abcDEF2345`, null)).toEqual([])
  })

  it('codes are checked before anything is looked up', () => {
    expect(wellFormedCode('abcDEF2345')).toBe(true)
    expect(wellFormedCode('abc/../xyz')).toBe(false)
    expect(wellFormedCode('ABCDEFGHIJ')).toBe(false) // I
  })
})

describe('the dialog choices', () => {
  it('members get up to 7 days and 25 uses; Never and No limit are the owner\'s', () => {
    expect(expiryChoices(false).map((c) => c.label)).toEqual(['1 hour', '1 day', '7 days'])
    expect(expiryChoices(true).map((c) => [c.label, c.value])).toEqual([['1 hour', 3600], ['1 day', 86400], ['7 days', 604800], ['Never', 0]])
    expect(usesChoices(false).map((c) => c.label)).toEqual(['1', '5', '25'])
    expect(usesChoices(true).map((c) => [c.label, c.value])).toEqual([['1', 1], ['5', 5], ['25', 25], ['No limit', 0]])
  })
})

describe('the card', () => {
  const p = { name: 'Alpha', join_text: undefined, for_user_name: 'kim' }
  it('reads per state', () => {
    expect(cardView('valid', p)).toEqual({ note: null, button: { label: 'Join Alpha', action: 'join' }, dead: false })
    expect(cardView('valid', { ...p, join_text: 'Hop in' }).button?.label).toBe('Hop in')
    expect(cardView('already_member', p)).toEqual({ note: "You're in", button: { label: 'Open', action: 'open' }, dead: false })
    expect(cardView('expired', p)).toMatchObject({ note: 'This invite has expired', button: null, dead: true })
    expect(cardView('used_up', p)).toMatchObject({ note: 'This invite has been used', button: null, dead: true })
    expect(cardView('for_someone_else', p)).toMatchObject({ note: 'For kim only', button: { label: 'Join Alpha', action: null }, dead: true })
    expect(cardView('banned', p)).toMatchObject({ note: "You can't join this server", dead: true })
    expect(cardView('not_found')).toMatchObject({ note: 'Invite not found', button: null, dead: true })
    expect(cardView('revoked', p).note).toBe('Invite not found')
  })

  it('a refused accept says what the invite is now', () => {
    expect(stateFromError('not found')).toBe('not_found')
    expect(stateFromError('This invite has expired.')).toBe('expired')
    expect(stateFromError('This invite has been used up.')).toBe('used_up')
    expect(stateFromError('This invite is for someone else.')).toBe('for_someone_else')
    expect(stateFromError("You can't join this server.")).toBe('banned')
    expect(stateFromError("You're already in this server.")).toBe('already_member')
    expect(stateFromError('slow down a little, then try again')).toBeNull()
  })

  it('counts and the Invites list', () => {
    expect(countsLine(3, 12)).toBe('3 online · 12 members')
    expect(countsLine(0, 1)).toBe('0 online · 1 member')
    const now = Date.parse('2026-11-18T12:00:00Z')
    expect(expiresText(undefined, now)).toBe('Never')
    expect(expiresText('2026-11-18T11:00:00Z', now)).toBe('Expired')
    expect(expiresText('2026-11-18T12:30:00Z', now)).toBe('in 30 minutes')
    expect(expiresText('2026-11-18T15:00:00Z', now)).toBe('in 3 hours')
    expect(expiresText('2026-11-25T12:00:00Z', now)).toBe('in 7 days')
    expect(usesText(2, 5)).toBe('2 / 5')
    expect(usesText(2)).toBe('2 / ∞')
  })
})

describe('InvitesChanged and BansChanged', () => {
  it('bump that server\'s count for your own devices only', () => {
    let s = { ...emptyState(), me: { id: 'U1' } } as never as ReturnType<typeof emptyState>
    s = applyEvent(s, { t: 'InvitesChanged', d: { user_id: 'U1', server_id: 'S1' } }, 0)
    s = applyEvent(s, { t: 'InvitesChanged', d: { user_id: 'U2', server_id: 'S1' } }, 0)
    s = applyEvent(s, { t: 'BansChanged', d: { user_id: 'U1', server_id: 'S2' } }, 0)
    s = applyEvent(s, { t: 'BansChanged', d: { user_id: 'U1', server_id: 'S2' } }, 0)
    expect(s.invitesRev).toEqual({ S1: 1 })
    expect(s.bansRev).toEqual({ S2: 2 })
  })
})
