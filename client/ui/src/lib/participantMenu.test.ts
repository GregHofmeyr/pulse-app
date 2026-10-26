// @vitest-environment jsdom
import { describe, expect, it } from 'vitest'
import { menuItems, menuPoint, type MenuCtx } from './participantMenu'

const base: MenuCtx = { self: false, live: false, watching: false, full: false, inCall: true, canGoLive: false, streamingSelf: false }
const ctx = (over: Partial<MenuCtx>): MenuCtx => ({ ...base, ...over })

describe('menuItems', () => {
  it('other_not_live', () => {
    expect(menuItems(ctx({}))).toEqual(['volume', 'local-mute', 'message'])
  })
  it('other_live_not_watching', () => {
    expect(menuItems(ctx({ live: true }))).toEqual(['volume', 'local-mute', 'message', 'watch'])
  })
  it('other_live_watching_in_grid', () => {
    expect(menuItems(ctx({ live: true, watching: true }))).toEqual(['volume', 'local-mute', 'message', 'stream-volume', 'stop-watching', 'full-view'])
  })
  it('other_live_watching_in_full_view_has_no_full_view_item', () => {
    expect(menuItems(ctx({ live: true, watching: true, full: true }))).toEqual(['volume', 'local-mute', 'message', 'stream-volume', 'stop-watching'])
  })
  it('self_in_call_can_go_live', () => {
    expect(menuItems(ctx({ self: true, canGoLive: true }))).toEqual(['mute', 'deafen', 'go-live', 'voice-settings'])
  })
  it('self_streaming', () => {
    expect(menuItems(ctx({ self: true, streamingSelf: true }))).toEqual(['mute', 'deafen', 'stop-streaming', 'voice-settings'])
  })
  it('self_not_in_call_has_no_stream_item', () => {
    expect(menuItems(ctx({ self: true, inCall: false }))).toEqual(['mute', 'deafen', 'voice-settings'])
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
