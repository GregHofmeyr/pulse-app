// @vitest-environment jsdom
// The question put before a stream action ends another stream of yours (#339).
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'

const resolveClash = vi.hoisted(() => vi.fn(async (_go: boolean) => {}))
vi.mock('../lib/stream/stream.svelte', () => ({ streamApi: { resolveClash } }))

import StreamClashDialog from './StreamClashDialog.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'
import type { StreamClash } from '../lib/stream/stream.svelte'

const sam = { id: 'U2', username: 'sam', display_name: 'sam', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }

let cmp: ReturnType<typeof mount> | undefined
function show(kind: StreamClash['kind']) {
  app.state = {
    ...emptyState(),
    people: { U2: { user: sam, online: true, last_seen_at: null } },
    channels: { A: { id: 'A', server_id: null, name: 'Lounge', kind: 'voice' }, B: { id: 'B', server_id: null, name: 'Games', kind: 'voice' } },
  } as never
  cmp = mount(StreamClashDialog, { target: document.body, props: { clash: { kind, liveIn: 'A', channelId: 'B', streamer: 'U2', run: () => {} } } })
  flushSync()
}
const button = (t: string) => [...document.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.trim() === t)

afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  resolveClash.mockClear()
})

describe('StreamClashDialog', () => {
  it('watching_while_live_says_where_and_offers_stop_and_watch', () => {
    show('watch')
    expect(document.querySelector('[role="dialog"]')?.textContent).toContain("You're live in Lounge. Stop streaming to watch sam in Games?")
    button('Stop & watch')!.click()
    expect(resolveClash).toHaveBeenCalledWith(true)
  })

  it('going_live_while_watching_says_where_and_cancel_changes_nothing', () => {
    show('live')
    expect(document.querySelector('[role="dialog"]')?.textContent).toContain("You're watching sam in Games. Stop watching to go live in Lounge?")
    expect(button('Stop & go live')).toBeDefined()
    button('Cancel')!.click()
    expect(resolveClash).toHaveBeenCalledWith(false)
  })
})
