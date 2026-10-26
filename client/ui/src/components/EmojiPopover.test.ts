// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/core', () => ({ invoke: async () => undefined }))

import EmojiPopover from './EmojiPopover.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'

let cmp: ReturnType<typeof mount> | undefined
beforeEach(() => {
  app.state = emptyState()
  Element.prototype.scrollIntoView = () => {}
})
afterEach(() => { if (cmp) unmount(cmp); cmp = undefined; document.body.innerHTML = '' })

it('passes_keep_through_to_onPick', () => {
  const onPick = vi.fn()
  const anchor = document.createElement('button')
  document.body.append(anchor)
  cmp = mount(EmojiPopover, { target: document.body, props: { anchor, onPick, onClose: () => {} } })
  flushSync()
  const cells = document.querySelectorAll<HTMLElement>('.cell')
  cells[0].dispatchEvent(new MouseEvent('click', { bubbles: true, shiftKey: true }))
  cells[1].click()
  expect(onPick.mock.calls.map((c) => c[1])).toEqual([{ keep: true }, { keep: false }])
})
