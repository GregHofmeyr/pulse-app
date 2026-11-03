// @vitest-environment jsdom
import { describe, expect, it } from 'vitest'
import { menuItems, menuPoint, type MenuCtx } from './participantMenu'

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
