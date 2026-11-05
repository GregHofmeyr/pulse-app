// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { fakeMatchMedia, FakeIntersectionObserver } from '../lib/testMotion'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))
vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)

import GroupMembers from './GroupMembers.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'

const user = (id: string, name: string) => ({ id, username: name, display_name: name, avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } })
const [alex, sam, kim, jo] = [user('U1', 'alex'), user('U2', 'sam'), user('U3', 'kim'), user('U4', 'jo')]
const p = (u: typeof alex, status: string) => ({ user: u, online: status !== 'offline', last_seen_at: null, status })

let cmp: ReturnType<typeof mount> | undefined
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

function show(people: Record<string, ReturnType<typeof p>>) {
  app.state = { ...emptyState(), me: alex, dmMembers: { G1: Object.keys(people) }, people } as never
  cmp = mount(GroupMembers, { target: document.body, props: { channelId: 'G1' } })
  flushSync()
}
const labels = () => [...document.querySelectorAll('.members .label')].map((l) => l.textContent?.trim())
const order = () =>
  [...document.querySelectorAll('.members .label, .members .row')].map((e) => (e.classList.contains('label') ? e.textContent?.trim() : e.querySelector('.name')!.textContent))

describe('GroupMembers Online and Offline sections (#170)', () => {
  it('Online (DND included) above Offline, each by name, with counts; offline rows dimmed', () => {
    show({ U1: p(alex, 'online'), U2: p(sam, 'dnd'), U3: p(kim, 'offline'), U4: p(jo, 'invisible') })
    expect(order()).toEqual(['Online — 2', 'alex', 'sam', 'Offline — 2', 'jo', 'kim'])
    for (const r of document.querySelectorAll<HTMLElement>('.members .row')) {
      expect(r.classList.contains('off')).toBe(['jo', 'kim'].includes(r.querySelector('.name')!.textContent!))
    }
  })

  it('every avatar has a dot; nobody else is ever drawn Invisible (#169, #171)', () => {
    show({ U1: p(alex, 'online'), U2: p(sam, 'invisible') })
    expect(document.querySelectorAll('.members .row .pres')).toHaveLength(2)
    expect(document.querySelector('.members .pres.invisible')).toBeNull()
  })

  it('a section with nobody has no heading', () => {
    show({ U1: p(alex, 'online'), U2: p(sam, 'online') })
    expect(labels()).toEqual(['Online — 2'])
  })
})
