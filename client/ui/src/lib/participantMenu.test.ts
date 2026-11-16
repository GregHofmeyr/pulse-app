// @vitest-environment jsdom
import { describe, expect, it, vi } from 'vitest'
import { memberVoiceItems, menuItems, menuOpener, menuPoint, personMenuAt, type MenuCtx } from './participantMenu'

describe('memberVoiceItems (#144)', () => {
  const c = { self: false, server: true, inVoice: false, muted: false, deafened: false }
  it('out of voice, nothing held: none', () => expect(memberVoiceItems(c)).toEqual([]))
  it('out of voice, held: just the undo for what is held', () => {
    expect(memberVoiceItems({ ...c, muted: true })).toEqual(['server-mute'])
    expect(memberVoiceItems({ ...c, deafened: true })).toEqual(['server-deafen'])
    expect(memberVoiceItems({ ...c, muted: true, deafened: true })).toEqual(['server-mute', 'server-deafen'])
  })
  it("in this server's voice: both, no disconnect", () => expect(memberVoiceItems({ ...c, inVoice: true })).toEqual(['server-mute', 'server-deafen']))
  it('never for yourself or outside a server', () => {
    expect(memberVoiceItems({ ...c, self: true, inVoice: true, muted: true })).toEqual([])
    expect(memberVoiceItems({ ...c, server: false, inVoice: true, muted: true })).toEqual([])
  })
})

const base: MenuCtx = { self: false, live: false, watching: false, full: false, inCall: true, canGoLive: false, streamingSelf: false }
const ctx = (over: Partial<MenuCtx>): MenuCtx => ({ ...base, ...over })

describe('menuItems', () => {
  it('other_not_live', () => {
    expect(menuItems(ctx({}))).toEqual(['volume', 'local-mute', 'sound-mute', 'message'])
  })
  it('other_live_not_watching', () => {
    expect(menuItems(ctx({ live: true }))).toEqual(['volume', 'local-mute', 'sound-mute', 'message', 'watch'])
  })
  it('other_live_watching_in_grid', () => {
    expect(menuItems(ctx({ live: true, watching: true }))).toEqual(['volume', 'local-mute', 'sound-mute', 'message', 'stream-volume', 'stop-watching', 'full-view'])
  })
  it('other_live_watching_in_full_view_has_no_full_view_item', () => {
    expect(menuItems(ctx({ live: true, watching: true, full: true }))).toEqual(['volume', 'local-mute', 'sound-mute', 'message', 'stream-volume', 'stop-watching'])
  })
  it('self_in_call_can_go_live', () => {
    expect(menuItems(ctx({ self: true, canGoLive: true }))).toEqual(['mute', 'deafen', 'go-live', 'voice-settings'])
  })
  it('self_streaming', () => {
    expect(menuItems(ctx({ self: true, streamingSelf: true }))).toEqual(['mute', 'deafen', 'stop-streaming', 'voice-settings'])
  })
  it('other_in_a_server_call_can_be_server_muted_deafened_and_disconnected', () => {
    expect(menuItems(ctx({ serverCall: true }))).toEqual(['volume', 'local-mute', 'sound-mute', 'message', 'server-mute', 'server-deafen', 'disconnect'])
    // whether or not you're in the call yourself
    expect(menuItems(ctx({ serverCall: true, inCall: false })).slice(-3)).toEqual(['server-mute', 'server-deafen', 'disconnect'])
    // after the stream items
    expect(menuItems(ctx({ serverCall: true, live: true })).slice(-4)).toEqual(['watch', 'server-mute', 'server-deafen', 'disconnect'])
  })
  it('never_on_yourself_or_in_dm_calls', () => {
    expect(menuItems(ctx({ serverCall: true, self: true }))).not.toContain('server-mute')
    expect(menuItems(ctx({ serverCall: false }))).not.toContain('disconnect')
  })
  it('self_not_in_call_has_no_stream_item', () => {
    expect(menuItems(ctx({ self: true, inCall: false }))).toEqual(['mute', 'deafen', 'voice-settings'])
  })
})

describe('sound-mute', () => {
  it('is_offered_for_others_after_local_mute_and_never_for_self', () => {
    expect(menuItems(ctx({}))).toContain('sound-mute')
    expect(menuItems(ctx({ self: true, canGoLive: true }))).not.toContain('sound-mute')
  })
})

describe('menuPoint', () => {
  const rect = { left: 40, bottom: 90, top: 50, right: 100, width: 60, height: 40 } as DOMRect
  const anchor = { getBoundingClientRect: () => rect } as HTMLElement
  it('a mouse event opens at the pointer', () => {
    expect(menuPoint(new MouseEvent('contextmenu', { clientX: 12, clientY: 34 }), anchor)).toEqual({ x: 12, y: 34 })
  })
  it('a key opens under the anchor', () => {
    expect(menuPoint(new KeyboardEvent('keydown', { key: 'ContextMenu' }), anchor)).toEqual({ x: 40, y: 90 })
  })
})

describe('personMenuAt (#349)', () => {
  const zone = () => {
    const z = document.createElement('span')
    z.innerHTML = '<button>alex</button>'
    document.body.append(z)
    return z
  }
  const right = () => new MouseEvent('contextmenu', { clientX: 5, clientY: 6, cancelable: true, bubbles: true })

  it('a right-click opens it at the pointer, claimed, focus going back to the button inside', () => {
    const z = zone()
    const e = right()
    const stop = vi.spyOn(e, 'stopPropagation')
    const m = personMenuAt(e, 'u1', 's1', z)
    expect(m).toEqual({ userId: 'u1', serverId: 's1', x: 5, y: 6, opener: z.querySelector('button') })
    expect(e.defaultPrevented).toBe(true)
    expect(stop).toHaveBeenCalled()
  })
  it('ContextMenu or Shift+F10 opens it; any other key is left alone', () => {
    const z = zone()
    expect(personMenuAt(new KeyboardEvent('keydown', { key: 'ContextMenu' }), 'u1', null, z)?.userId).toBe('u1')
    expect(personMenuAt(new KeyboardEvent('keydown', { key: 'F10', shiftKey: true }), 'u1', null, z)?.userId).toBe('u1')
    const other = new KeyboardEvent('keydown', { key: 'Enter', cancelable: true })
    expect(personMenuAt(other, 'u1', null, z)).toBeNull()
    expect(other.defaultPrevented).toBe(false)
  })
  it('no one to show (a system line): nothing opens and the event is left alone', () => {
    const e = right()
    expect(personMenuAt(e, null, null, zone())).toBeNull()
    expect(e.defaultPrevented).toBe(false)
  })
  it('a zone that is a button hands focus back to itself; a row places as rowMenuAt', () => {
    const b = document.createElement('button')
    document.body.append(b)
    expect(menuOpener(b)).toBe(b)
    const row = document.createElement('div')
    row.innerHTML = '<span class="name">sam</span>'
    document.body.append(row)
    expect(personMenuAt(right(), 'u2', null, row, { row: true })?.opener).toBe(row.querySelector('.name'))
  })
})
