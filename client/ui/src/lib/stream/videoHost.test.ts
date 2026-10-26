// @vitest-environment jsdom
// The store's <video> moves between hosts (grid tile, full view, mini-preview) without restarting.
import { beforeEach, describe, expect, it, vi } from 'vitest'

const store = vi.hoisted(() => ({ el: null as HTMLVideoElement | null }))
vi.mock('./stream.svelte', () => ({ videoOf: () => store.el }))

import { hostVideo } from './videoHost'

beforeEach(() => {
  store.el = document.createElement('video')
  vi.spyOn(HTMLMediaElement.prototype, 'play').mockResolvedValue()
})

describe('hostVideo', () => {
  it('moving_between_hosts_keeps_the_element', () => {
    const a = document.createElement('div')
    const b = document.createElement('div')
    const unmountA = hostVideo('c1', 'other')(a)
    expect(a.firstChild).toBe(store.el)
    const unmountB = hostVideo('c1', 'other')(b)
    if (typeof unmountA === 'function') unmountA() // A unmounts after B took the element
    expect(b.firstChild).toBe(store.el)
    expect(store.el!.parentNode).toBe(b)
    if (typeof unmountB === 'function') unmountB()
    expect(store.el!.parentNode).toBeNull()
  })

  it('plays_a_paused_element_once_hosted', () => {
    hostVideo('c1', 'other')(document.createElement('div'))
    expect(HTMLMediaElement.prototype.play).toHaveBeenCalled()
  })

  it('hosts_nothing_without_a_watch', () => {
    store.el = null
    const node = document.createElement('div')
    hostVideo('c1', 'other')(node)
    expect(node.childNodes).toHaveLength(0)
  })
})
