import { describe, expect, it, vi } from 'vitest'
import { allActions, matchActions, type ActionContext } from './quickActions'
import type { Sound } from './protocol/Sound'

const sound = (id: string, name: string): Sound => ({ id, server_id: 's1', name, emoji: null, emoji_id: null, media_hash: `h-${id}`, volume: 100, duration_ms: 1000, added_by: null, created_at: '', plays: 0 })

const ctx = (o: Partial<ActionContext> = {}): ActionContext => ({
  call: null,
  muted: false,
  deafened: false,
  voiceChannels: [{ id: 'v1', name: 'Lounge', server: 'Game Night' }, { id: 'v2', name: 'Raid', server: 'Game Night' }],
  sounds: [sound('a', 'Airhorn'), sound('b', 'Bruh')],
  goLiveOff: null,
  join: vi.fn(),
  play: vi.fn(),
  toggleMute: vi.fn(),
  toggleDeafen: vi.fn(),
  leave: vi.fn(),
  goLive: vi.fn(),
  openSettings: vi.fn(),
  openInbox: vi.fn(),
  ...o,
})
const inCall = { call: { channelId: 'v1', name: 'Lounge', connected: true } }
const ids = (c: ActionContext) => allActions(c).map((a) => a.id)

describe('the action registry', () => {
  it('out of a call: join a voice channel and Settings pages, no call controls or sounds', () => {
    const list = ids(ctx())
    expect(list).toEqual(expect.arrayContaining(['join:v1', 'join:v2', 'settings', 'settings:keybinds']))
    for (const id of ['mute', 'deafen', 'leave', 'golive', 'play:a']) expect(list).not.toContain(id)
  })
  it('in a call: mute, deafen, Go Live, leave and the sounds; no joining the call you are in', () => {
    const list = ids(ctx(inCall))
    expect(list).toEqual(expect.arrayContaining(['mute', 'deafen', 'golive', 'leave', 'play:a', 'play:b', 'join:v2']))
    expect(list).not.toContain('join:v1')
  })
  it('labels follow the state: Unmute / Undeafen when on', () => {
    const list = allActions(ctx({ ...inCall, muted: true, deafened: true }))
    expect(list.find((a) => a.id === 'mute')!.label).toBe('Unmute')
    expect(list.find((a) => a.id === 'deafen')!.label).toBe('Undeafen')
    const off = allActions(ctx(inCall))
    expect(off.find((a) => a.id === 'mute')!.label).toBe('Mute')
  })
  it('Notes (#308): opens your notes until they exist (then they are a place of their own)', () => {
    expect(ids(ctx())).not.toContain('notes')
    const openNotes = vi.fn()
    const c = ctx({ openNotes })
    const notes = allActions(c).find((a) => a.id === 'notes')!
    expect(notes.label).toBe('Notes')
    expect(matchActions(allActions(c), 'notes', 50, true).map((a) => a.id)).toContain('notes')
    notes.run(c)
    expect(openNotes).toHaveBeenCalled()
  })
  it('no Go Live where it is off (Linux, not connected)', () => {
    expect(ids(ctx({ ...inCall, goLiveOff: "Streaming from Linux isn't supported yet" }))).not.toContain('golive')
  })
  it('each runs through the context', () => {
    const c = ctx(inCall)
    const run = (id: string) => allActions(c).find((a) => a.id === id)!.run(c)
    run('join:v2')
    expect(c.join).toHaveBeenCalledWith('v2')
    run('play:b')
    expect(c.play).toHaveBeenCalledWith(expect.objectContaining({ id: 'b' }))
    run('mute')
    expect(c.toggleMute).toHaveBeenCalled()
    run('deafen')
    expect(c.toggleDeafen).toHaveBeenCalled()
    run('leave')
    expect(c.leave).toHaveBeenCalled()
    run('golive')
    expect(c.goLive).toHaveBeenCalled()
    run('settings:keybinds')
    expect(c.openSettings).toHaveBeenCalledWith('keybinds')
    run('settings')
    expect(c.openSettings).toHaveBeenCalledWith('voice')
  })
  it('Inbox opens the inbox (#306), found after > and close by its name or "mentions"', () => {
    const c = ctx()
    const list = allActions(c)
    list.find((a) => a.id === 'inbox')!.run(c)
    expect(c.openInbox).toHaveBeenCalled()
    expect(matchActions(list, 'inbox')[0].id).toBe('inbox')
    expect(matchActions(list, 'inb', 50, true)[0]?.id).toBe('inbox')
    expect(matchActions(list, 'mentions')[0].id).toBe('inbox')
  })
  it('ids are unique', () => {
    const list = ids(ctx(inCall))
    expect(new Set(list).size).toBe(list.length)
  })
})

describe('matchActions', () => {
  const list = allActions(ctx(inCall))
  it('finds by label or keyword, best first', () => {
    expect(matchActions(list, 'join raid')[0].id).toBe('join:v2')
    expect(matchActions(list, 'raid')[0].id).toBe('join:v2')
    expect(matchActions(list, 'air')[0].id).toBe('play:a')
    expect(matchActions(list, 'mute')[0].id).toBe('mute')
    expect(matchActions(list, 'keyb')[0].id).toBe('settings:keybinds')
    expect(matchActions(list, 'hang up')[0].id).toBe('leave')
  })
  it('under the places (close): only a word starting with the query, in what sets the action apart', () => {
    const close = (q: string) => matchActions(list, q, 5, true).map((a) => a.id)
    // no family by its shared verb or word, and nothing merely containing the letter
    expect(close('j')).toEqual([])
    expect(close('g')).toEqual(['golive'])
    expect(close('voice')).toEqual(['settings:voice'])
    expect(close('sound')).toEqual([])
    expect(close('set')).toEqual(['settings'])
    // but by its own name
    expect(close('rai')).toEqual(['join:v2'])
    expect(close('air')).toEqual(['play:a'])
    expect(close('keyb')).toEqual(['settings:keybinds'])
    expect(close('mute')).toEqual(['mute'])
    // after `>` the family words list it all
    expect(matchActions(list, 'sound').map((a) => a.id)).toEqual(['play:a', 'play:b'])
  })
  it('everything (in registry order) for nothing typed, capped', () => {
    expect(matchActions(list, '').map((a) => a.id)).toEqual(list.map((a) => a.id))
    expect(matchActions(list, '', 3)).toHaveLength(3)
    expect(matchActions(list, 'zzzz')).toEqual([])
  })
})
