// @vitest-environment jsdom
// #288: a Spotify link in a voice channel's chat, while you're in its call, offers to put it on the stage.
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: async () => undefined }))

import Composer, { forgetStageNudges } from './Composer.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import { positionAt, serialize } from '../lib/composerDom'

let cmp: ReturnType<typeof mount> | undefined
const onSend = vi.fn()
const share = vi.fn(async (_url: string) => {})
function show(extra: Record<string, unknown> = {}) {
  if (cmp) unmount(cmp)
  document.body.innerHTML = ''
  const me = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
  const sam = { ...me, id: 'U2', username: 'sam', display_name: 'sam' }
  app.state = { ...emptyState(), me, servers: [{ id: 'S1', name: 'Main', icon_hash: null }], members: { S1: [{ user: me, nickname: null }] } }
  cmp = mount(Composer, {
    target: document.body,
    props: {
      placeholder: 'Message', draftKey: `K${Math.random()}`, replyingTo: null, onCancelReply: () => {}, onSend, onTyping: () => {},
      mentionables: [{ id: sam.id, username: 'sam', name: 'sam', alts: [] }], ...extra,
    },
  })
  flushSync()
  return document.querySelector<HTMLElement>('[role="textbox"]')!
}
const withNudge = (up: string[] = []) => show({ stageNudge: { share, up } })
function type(box: HTMLElement, text: string) {
  box.replaceChildren(document.createTextNode(text))
  box.focus()
  const p = positionAt(box, text.length)
  const r = document.createRange()
  r.setStart(p.node, p.offset)
  r.collapse(true)
  getSelection()!.removeAllRanges()
  getSelection()!.addRange(r)
  document.dispatchEvent(new Event('selectionchange'))
  box.dispatchEvent(new InputEvent('input', { bubbles: true }))
  flushSync()
}
const enter = (box: HTMLElement) => {
  box.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }))
  flushSync()
}
const ID = '4uLU6hMCjMI75M1A2tKUQC'
const link = (n = 0) => `https://open.spotify.com/track/${ID.slice(0, 21)}${n}?si=abc`
const chip = () => document.querySelector<HTMLElement>('.stage-nudge')
const shareBtn = () => [...chip()!.querySelectorAll('button')].find((b) => b.textContent?.trim() === 'Share to the stage?')!

beforeEach(() => {
  onSend.mockClear()
  share.mockClear()
  forgetStageNudges()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

describe('Composer: Share to the stage? nudge', () => {
  it('shows a chip above the box, in the suggestion style, for a Spotify link typed or pasted; a click shares that link', async () => {
    const box = withNudge()
    type(box, 'nice')
    expect(chip()).toBeNull()
    type(box, `this one ${link()} is great`)
    const c = chip()!
    expect(c.classList.contains('mentions')).toBe(true)
    expect(c.classList.contains('menu-surface')).toBe(true)
    expect(c.textContent).toContain('Share to the stage?')
    expect(c.querySelector('[aria-label="Spotify"]')).not.toBeNull()
    shareBtn().click()
    await tick()
    flushSync()
    expect(share).toHaveBeenCalledWith(link())
    expect(chip()).toBeNull()
    // the message is still there to send as it is
    expect(serialize(box)).toBe(`this one ${link()} is great`)
    enter(box)
    expect(onSend).toHaveBeenCalledWith(`this one ${link()} is great`, [])
  })

  it('a box holding only the link empties once it is shared: the share posts it in the chat already', async () => {
    const box = withNudge()
    type(box, ` ${link(5)} `)
    shareBtn().click()
    for (let i = 0; i < 3; i++) await tick()
    flushSync()
    expect(share).toHaveBeenCalledWith(link(5))
    expect(serialize(box)).toBe('')
  })

  it('✕ dismisses it for that link for the session; another copy of it stays quiet, another link still asks', () => {
    const box = withNudge()
    type(box, link())
    chip()!.querySelector<HTMLElement>('button[aria-label="Dismiss"]')!.click()
    flushSync()
    expect(chip()).toBeNull()
    type(box, `again ${link().replace('si=abc', 'si=xyz')}`)
    expect(chip()).toBeNull()
    type(box, link(1))
    expect(chip()).not.toBeNull()
  })

  it('once per link: sending it without sharing doesn\'t ask again for it', () => {
    let box = withNudge()
    type(box, link())
    enter(box)
    expect(onSend).toHaveBeenCalled()
    box = withNudge()
    type(box, link())
    expect(chip()).toBeNull()
  })

  it('not for a link already on the stage, nor where there is no stage to share to, nor over a suggestion list', () => {
    let box = withNudge([link().replace('si=abc', 'si=other')])
    type(box, link())
    expect(chip()).toBeNull()
    box = show()
    type(box, link(2))
    expect(chip()).toBeNull()
    box = withNudge()
    type(box, `${link(3)} @sa`)
    expect(document.querySelector('[aria-label="Mention someone"]')).not.toBeNull()
    expect(chip()).toBeNull()
  })

  it('a share that fails says why and keeps the chip', async () => {
    share.mockRejectedValueOnce('not in the call')
    const box = withNudge()
    type(box, link(4))
    shareBtn().click()
    for (let i = 0; i < 3; i++) await tick()
    flushSync()
    expect(chip()!.querySelector('[role=alert]')!.textContent).toContain('not in the call')
    expect(box).toBeTruthy()
  })
})
