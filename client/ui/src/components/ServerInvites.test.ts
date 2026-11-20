// @vitest-environment jsdom
// Server invites and bans, the app side (#80): the Invite dialog (a ready link on the defaults, the owner's extra
// choices, Copy, a fresh link per change, direct invites), "Invite to server ›" on a person's menu, the invite card in
// each state (looked up once it's on screen, once per code), invite links in a message, Server settings › Invites and
// › Bans following InvitesChanged and BansChanged, and Ban on a member's menu.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fakeMatchMedia, FakeIntersectionObserver } from '../lib/testMotion'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))
vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)

import InviteDialog from './InviteDialog.svelte'
import ServerInviteCard from './ServerInviteCard.svelte'
import MemberMenu from './MemberMenu.svelte'
import MessageItem from './MessageItem.svelte'
import ServerSettings from './ServerSettings.svelte'
import ServerActionDialog from './ServerActionDialog.svelte'
import { app, resetState, ui } from '../lib/store.svelte'
import { applyEvent, emptyState } from '../lib/state'
import { forgetInvitePreviews, invitePreviews } from '../lib/invitePreviews.svelte'
import type { InvitePreview } from '../lib/protocol/InvitePreview'
import type { Message } from '../lib/protocol/Message'
import type { ServerInvite } from '../lib/protocol/ServerInvite'

const user = (id: string, name: string) => ({ id, username: name, display_name: name, avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } })
const alex = user('U1', 'alex')
const sam = user('U2', 'sam')
const kim = user('U3', 'kim')
const jo = user('U4', 'jo')
const ORIGIN = 'https://pulsechat.co.za'
const CODE = 'abcDEF2345'

const invite = (over: Partial<ServerInvite> = {}): ServerInvite =>
  ({ code: CODE, server_id: 'S1', created_by: 'U1', created_at: '2026-11-18T10:00:00Z', expires_at: '2099-01-01T00:00:00Z', max_uses: 1, uses: 0, state: 'valid', ...over })
const preview = (over: Partial<InvitePreview> = {}): InvitePreview =>
  ({ code: CODE, server_id: 'S2', name: 'Beta', member_count: 12, online_count: 3, faces: [sam], inviter_id: 'U2', inviter_name: 'sam', state: 'valid', ...over })

/** alex owns Alpha (with sam in it); Beta is sam's, alex isn't in it; kim and jo are known from elsewhere. */
function world(owner = 'U1') {
  app.state = {
    ...emptyState(),
    me: alex,
    servers: [
      { id: 'S1', name: 'Alpha', icon_hash: null, owner_id: owner },
      { id: 'S2', name: 'Beta', icon_hash: null, owner_id: 'U2' },
      { id: 'S3', name: 'Gamma', icon_hash: null, owner_id: 'U1' },
    ],
    members: {
      S1: [{ user: alex, nickname: null }, { user: sam, nickname: null }],
      S2: [{ user: sam, nickname: null }],
      S3: [{ user: alex, nickname: null }, { user: kim, nickname: null }],
    },
    people: Object.fromEntries([alex, sam, kim, jo].map((u) => [u.id, { user: u, online: true, last_seen_at: null, status: 'online' }])),
  } as never
}

let cmp: ReturnType<typeof mount> | undefined
const settle = async () => {
  for (let i = 0; i < 6; i++) await Promise.resolve()
  flushSync()
}
beforeEach(() => {
  // an opened invite pack stays open on this device (#511): each test starts with none opened
  localStorage.clear()
  resetState()
  invoke.mockReset()
  invoke.mockResolvedValue(undefined)
  FakeIntersectionObserver.all = []
  world()
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  vi.useRealTimers()
})
const calls = (cmd: string) => invoke.mock.calls.filter((c) => c[0] === cmd).map((c) => c[1] as Record<string, unknown>)

describe('the Invite dialog', () => {
  let made = 0
  const answer = (cmd: string, args?: unknown) => {
    if (cmd === 'server_origin') return ORIGIN
    if (cmd === 'create_server_invite') return invite({ code: `code${String(++made).padStart(6, 'a')}` })
    return undefined
  }
  const open = async () => {
    made = 0
    invoke.mockImplementation(async (cmd: string, args?: unknown) => answer(cmd, args))
    cmp = mount(InviteDialog, { target: document.body, props: { serverId: 'S1', onClose: vi.fn() } })
    flushSync()
    await settle()
  }
  const linkField = () => document.querySelector<HTMLInputElement>('input[aria-label="Invite link"]')!
  const seg = (label: string) => [...document.querySelectorAll<HTMLElement>(`[role=radiogroup][aria-label="${label}"] [role=radio]`)]
  const picked = (label: string) => seg(label).find((b) => b.getAttribute('aria-checked') === 'true')?.textContent

  it('opens on a ready link with a day and one use', async () => {
    await open()
    expect(document.querySelector('[role=dialog]')!.getAttribute('aria-label')).toBe('Invite to Alpha')
    expect(calls('create_server_invite')).toEqual([{ serverId: 'S1', request: { max_age_secs: 86400, max_uses: 1, join_text: undefined } }])
    expect(linkField().value).toBe('https://pulsechat.co.za/i/codeaaaaa1')
    expect(picked('Expires')).toBe('1 day')
    expect(picked('Uses')).toBe('1')
  })

  it('Never and No limit are only offered to the owner', async () => {
    world('U2')
    await open()
    expect(seg('Expires').map((b) => b.textContent)).toEqual(['1 hour', '1 day', '7 days'])
    expect(seg('Uses').map((b) => b.textContent)).toEqual(['1', '5', '25'])
    unmount(cmp!)
    document.body.innerHTML = ''
    world('U1')
    await open()
    expect(seg('Expires').map((b) => b.textContent)).toContain('Never')
    expect(seg('Uses').map((b) => b.textContent)).toContain('No limit')
  })

  it('a change is saved onto the same link: its code never changes and nothing is revoked', async () => {
    await open()
    seg('Expires').find((b) => b.textContent === '7 days')!.click()
    await settle()
    expect(calls('create_server_invite')).toHaveLength(1)
    expect(calls('update_invite')).toEqual([{ code: 'codeaaaaa1', request: { max_age_secs: 604800, max_uses: 1, join_text: '' } }])
    expect(linkField().value).toMatch(/codeaaaaa1$/)
    seg('Uses').find((b) => b.textContent === '5')!.click()
    await settle()
    expect(calls('update_invite').at(-1)).toEqual({ code: 'codeaaaaa1', request: { max_age_secs: 604800, max_uses: 5, join_text: '' } })
    expect(calls('revoke_invite')).toEqual([])
    expect(linkField().value).toMatch(/codeaaaaa1$/)
  })

  it('the button text waits for a pause, then is saved onto the link', async () => {
    await open()
    vi.useFakeTimers()
    const field = document.querySelector<HTMLInputElement>('input[placeholder="Join Alpha"]')!
    expect(field.maxLength).toBe(32)
    field.value = 'Come hang'
    field.dispatchEvent(new Event('input', { bubbles: true }))
    flushSync()
    expect(calls('update_invite')).toHaveLength(0)
    vi.advanceTimersByTime(700)
    vi.useRealTimers()
    await settle()
    expect(calls('create_server_invite')).toHaveLength(1)
    expect(calls('update_invite').at(-1)).toEqual({ code: 'codeaaaaa1', request: { max_age_secs: 86400, max_uses: 1, join_text: 'Come hang' } })
  })

  it('typing, leaving the field, then Copy: the same link, copied inside the click, and kept on close', async () => {
    await open()
    let release: (() => void) | undefined
    // the save is still on its way when Copy is pressed
    invoke.mockImplementation(async (cmd: string, args?: unknown) => {
      if (cmd === 'update_invite') return new Promise((r) => (release = () => r(invite({ code: 'codeaaaaa1', join_text: 'Come hang' }))))
      return answer(cmd, args)
    })
    const field = document.querySelector<HTMLInputElement>('input[placeholder="Join Alpha"]')!
    field.value = 'Come hang'
    field.dispatchEvent(new Event('input', { bubbles: true }))
    field.dispatchEvent(new FocusEvent('blur'))
    field.dispatchEvent(new FocusEvent('focusout', { bubbles: true }))
    flushSync()
    expect(calls('update_invite')).toHaveLength(1)
    const writeText = vi.fn(async () => {})
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true })
    const copy = [...document.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.includes('Copy'))!
    expect(copy.disabled).toBe(false)
    copy.click()
    // synchronously, in the click itself
    expect(writeText).toHaveBeenCalledWith('https://pulsechat.co.za/i/codeaaaaa1')
    release?.()
    await settle()
    expect(linkField().value).toMatch(/codeaaaaa1$/)
    expect(calls('create_server_invite')).toHaveLength(1)
    unmount(cmp!)
    cmp = undefined
    await settle()
    expect(calls('revoke_invite')).toEqual([])
  })

  it('lists people you know who are not in it; Invite sends a direct invite and becomes Sent', async () => {
    await open()
    const rows = () => [...document.querySelectorAll<HTMLElement>('[data-person]')]
    expect(rows().map((r) => r.dataset.person)).toEqual(['U4', 'U3']) // jo, kim; not you, not sam (in it)
    rows()[1].querySelector('button')!.click()
    await settle()
    expect(calls('create_server_invite').at(-1)).toEqual({ serverId: 'S1', request: { for_user: 'U3' } })
    expect(rows()[1].querySelector('button')!.textContent).toBe('Sent')
    expect(rows()[1].querySelector('button')!.disabled).toBe(true)
  })

  it('closing revokes the link nobody copied', async () => {
    await open()
    unmount(cmp!)
    cmp = undefined
    await settle()
    expect(calls('revoke_invite')).toEqual([{ code: 'codeaaaaa1' }])
  })
})

describe('Invite to server › on a person\'s menu', () => {
  const open = (userId: string) => {
    const opener = document.createElement('button')
    document.body.append(opener)
    cmp = mount(MemberMenu, { target: document.body, props: { serverId: 'S1', userId, x: 10, y: 10, opener, onClose: () => {} } })
    flushSync()
  }
  const flyout = () => document.querySelector<HTMLElement>('[role=menu][aria-label="Invite to a server"]')
  it('lists the servers you are in, the ones they are in greyed; picking one sends the invite and says so', async () => {
    open('U3')
    const item = document.querySelector<HTMLElement>('[data-item="invite-server"]')!
    item.click()
    flushSync()
    const choices = [...flyout()!.querySelectorAll<HTMLElement>('[role=menuitem]')]
    expect(choices.map((c) => c.querySelector('.name')?.textContent)).toEqual(['Alpha', 'Gamma']) // not Beta: you're not in it
    expect(choices.map((c) => c.getAttribute('aria-disabled'))).toEqual([null, 'true']) // kim's in Gamma
    expect(choices[0].querySelector('.chip')).not.toBeNull()
    choices[1].click()
    await settle()
    expect(calls('create_server_invite')).toEqual([])
    choices[0].click()
    await settle()
    expect(calls('create_server_invite')).toEqual([{ serverId: 'S1', request: { for_user: 'U3' } }])
    expect(ui.notice).toBe('Invite sent')
  })
  it('not for yourself', () => {
    open('U1')
    expect(document.querySelector('[data-item="invite-server"]')).toBeNull()
  })
})

describe('Ban on a member\'s menu', () => {
  const open = (userId: string) => {
    const opener = document.createElement('button')
    document.body.append(opener)
    cmp = mount(MemberMenu, { target: document.body, props: { serverId: 'S1', userId, x: 10, y: 10, opener, onClose: () => {} } })
    flushSync()
  }
  it('the owner bans after a confirm', async () => {
    open('U2')
    document.querySelector<HTMLElement>('[data-item="ban"]')!.click()
    expect(ui.serverAction).toEqual({ kind: 'ban', serverId: 'S1', userId: 'U2' })
    unmount(cmp!)
    document.body.innerHTML = ''
    cmp = mount(ServerActionDialog, { target: document.body, props: { action: ui.serverAction!, onClose: vi.fn() } })
    flushSync()
    expect(document.querySelector('.modal-title')!.textContent).toBe('Ban sam')
    ;[...document.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent === 'Ban')!.click()
    await settle()
    expect(calls('set_server_ban')).toEqual([{ serverId: 'S1', userId: 'U2', on: true }])
  })
  it('only the owner', () => {
    world('U2')
    open('U2')
    expect(document.querySelector('[data-item="ban"]')).toBeNull()
  })
})

describe('the invite card', () => {
  const show = async (answer: () => unknown) => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd !== 'invite_preview') return undefined
      return answer()
    })
    cmp = mount(ServerInviteCard, { target: document.body, props: { code: CODE } })
    flushSync()
    const card = document.querySelector<HTMLElement>('[data-server-invite]')!
    FakeIntersectionObserver.set(card, true)
    await settle()
    return card
  }
  const button = () => document.querySelector<HTMLButtonElement>('.join')
  /** Open the sealed pack (#511) as Enter on it does, then let the rip finish. */
  const openPack = async () => {
    vi.useFakeTimers()
    document.querySelector<HTMLElement>('.pack')!.dispatchEvent(new MouseEvent('click', { bubbles: true, detail: 0 }))
    vi.advanceTimersByTime(700)
    vi.useRealTimers()
    flushSync()
  }
  const reject = (message: string) => () => Promise.reject({ message })

  it('looks the code up once it is on screen, and only once per code', async () => {
    invoke.mockImplementation(async (cmd: string) => (cmd === 'invite_preview' ? preview() : undefined))
    cmp = mount(ServerInviteCard, { target: document.body, props: { code: CODE } })
    flushSync()
    await settle()
    expect(calls('invite_preview')).toEqual([])
    FakeIntersectionObserver.set(document.querySelector('[data-server-invite]')!, true)
    await settle()
    expect(calls('invite_preview')).toEqual([{ code: CODE }])
    unmount(cmp)
    cmp = mount(ServerInviteCard, { target: document.body, props: { code: CODE } })
    flushSync()
    FakeIntersectionObserver.set(document.querySelectorAll('[data-server-invite]')[0], true)
    await settle()
    expect(calls('invite_preview')).toHaveLength(1)
  })

  it('valid: sealed until opened, then the server, who it is from, its counts and Join; joining goes there', async () => {
    const card = await show(() => preview({ join_text: 'Hop in' }))
    // sealed: Join waits inside the pack
    expect(card.dataset.stage).toBe('sealed')
    expect(button()!.disabled).toBe(true)
    await openPack()
    expect(card.dataset.stage).toBe('open')
    expect(card.textContent).toContain('Beta')
    expect(card.textContent).toContain('from sam · 3 online · 12 members')
    expect(card.textContent).toContain('✦ MYTHICAL PULL ✦')
    expect(button()!.textContent).toBe('Hop in')
    invoke.mockImplementation(async (cmd: string) => (cmd === 'accept_invite' ? app.state.servers[1] : undefined))
    button()!.click()
    await settle()
    expect(calls('accept_invite')).toEqual([{ code: CODE }])
    expect(ui.goServer).toBe('S2')
    expect(button()!.textContent).toBe('Open')
  })

  it('the pack (#511): its sender sees the card open; once opened it stays open on this device', async () => {
    let card = await show(() => preview({ inviter_id: 'U1', inviter_name: 'alex' }))
    expect(card.dataset.stage).toBe('open')
    expect(document.querySelector('.pack')).toBeNull()
    expect(card.textContent).toContain('from you')
    unmount(cmp!)
    cmp = undefined
    document.body.innerHTML = ''
    forgetInvitePreviews()
    card = await show(() => preview())
    expect(card.dataset.stage).toBe('sealed')
    expect(document.querySelector('.pack')!.getAttribute('aria-label')).toContain('press Enter')
    await openPack()
    expect(localStorage.getItem(`pulse.invitePack.${CODE}`)).toBe('1')
    unmount(cmp!)
    cmp = undefined
    document.body.innerHTML = ''
    forgetInvitePreviews()
    card = await show(() => preview())
    expect(card.dataset.stage).toBe('open')
  })

  it('the pack: a dead invite skips it', async () => {
    let card = await show(() => preview({ state: 'expired' }))
    expect(card.dataset.stage).toBe('open')
    expect(card.classList.contains('dead')).toBe(true)
    expect(document.querySelector('.rarity')).toBeNull()
  })

  // #538: Windows "Animation effects" off used to skip the pack; now it arrives sealed, still, and opens at once
  describe('with reduced motion', () => {
    beforeEach(() => vi.stubGlobal('matchMedia', fakeMatchMedia(true).matchMedia))
    afterEach(() => vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia))
    const fresh = () => {
      unmount(cmp!)
      cmp = undefined
      document.body.innerHTML = ''
      forgetInvitePreviews()
    }

    it('it arrives sealed; a click opens it at once, with no rip, burst or rise, and it stays open', async () => {
      let card = await show(() => preview())
      expect(card.dataset.stage).toBe('sealed')
      const pack = document.querySelector<HTMLElement>('.pack')!
      expect(pack.getAttribute('aria-label')).toContain('Click or press Enter')
      expect(card.querySelector('.hint')!.textContent).toBe('Click to open it')
      pack.dispatchEvent(new MouseEvent('click', { bubbles: true, detail: 1 }))
      flushSync()
      expect(card.dataset.stage).toBe('open')
      expect(card.classList.contains('opening')).toBe(false)
      expect(card.querySelector('.burst')).toBeNull()
      expect(document.querySelector('.pack')).toBeNull()
      expect(button()!.disabled).toBe(false)
      expect(localStorage.getItem(`pulse.invitePack.${CODE}`)).toBe('1')
      fresh()
      card = await show(() => preview())
      expect(card.dataset.stage).toBe('open')
    })

    it('Enter opens it at once', async () => {
      const card = await show(() => preview())
      document.querySelector<HTMLElement>('.pack')!.dispatchEvent(new MouseEvent('click', { bubbles: true, detail: 0 }))
      flushSync()
      expect(card.dataset.stage).toBe('open')
      expect(card.querySelector('.burst')).toBeNull()
    })

    it('a short drag across the top opens it at once', async () => {
      const card = await show(() => preview())
      const pack = document.querySelector<HTMLElement>('.pack')!
      pack.getBoundingClientRect = () => ({ left: 0, top: 0, width: 260, height: 400, right: 260, bottom: 400, x: 0, y: 0, toJSON: () => ({}) })
      const at = (type: string, x: number) => pack.dispatchEvent(Object.assign(new MouseEvent(type, { bubbles: true, clientX: x, clientY: 20 }), { pointerId: 1 }))
      at('pointerdown', 100)
      flushSync()
      expect(card.dataset.stage).toBe('sealed')
      at('pointermove', 150) // about 19% of the width
      flushSync()
      expect(card.dataset.stage).toBe('open')
      expect(card.querySelector('.burst')).toBeNull()
    })

    it('its sender still sees it open, and a dead invite is still the greyed card', async () => {
      let card = await show(() => preview({ inviter_id: 'U1', inviter_name: 'alex' }))
      expect(card.dataset.stage).toBe('open')
      expect(document.querySelector('.pack')).toBeNull()
      fresh()
      card = await show(() => preview({ state: 'used_up' }))
      expect(card.dataset.stage).toBe('open')
      expect(card.classList.contains('dead')).toBe(true)
    })
  })

  it('without join text: Join <server>', async () => {
    await show(() => preview())
    expect(button()!.textContent).toBe('Join Beta')
  })

  it("already in: You're in, and Open goes there", async () => {
    const card = await show(() => preview({ server_id: 'S1', name: 'Alpha', state: 'already_member' }))
    expect(card.textContent).toContain("You're in")
    button()!.click()
    expect(ui.goServer).toBe('S1')
  })

  it.each([
    ['expired', 'This invite has expired'],
    ['used_up', 'This invite has been used'],
    ['banned', "You can't join this server"],
  ] as const)('%s: greyed, no Join', async (state, note) => {
    const card = await show(() => preview({ state }))
    expect(card.classList.contains('dead')).toBe(true)
    expect(card.querySelector('[data-note]')!.textContent).toBe(note)
    expect(button()).toBeNull()
  })

  it('for someone else: For <name> only, Join disabled', async () => {
    const card = await show(() => preview({ state: 'for_someone_else', for_user: 'U3', for_user_name: 'kim' }))
    expect(card.textContent).toContain('For kim only')
    expect(button()!.disabled).toBe(true)
  })

  it('not found (unknown or revoked): Invite not found with a ? icon', async () => {
    const card = await show(reject('not found'))
    expect(card.textContent).toContain('Invite not found')
    expect(card.querySelector('.unknown')!.textContent).toBe('?')
    expect(button()).toBeNull()
  })

  it('a refused join says what the invite is now', async () => {
    await show(() => preview())
    await openPack()
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'accept_invite') throw { message: 'This invite has expired.' }
      return undefined
    })
    button()!.click()
    await settle()
    expect(document.querySelector('[data-note]')!.textContent).toBe('This invite has expired')
    expect(calls('invite_preview')).toHaveLength(1)
  })

  it('a failed lookup is not asked again in a loop', async () => {
    await show(reject('slow down a little, then try again'))
    expect(document.body.textContent).toContain("Couldn't load this invite right now")
    flushSync()
    await settle()
    expect(calls('invite_preview')).toHaveLength(1)
    expect(invitePreviews.byCode[CODE].status).toBe('error')
  })
})

describe('a message', () => {
  const msg = (over: Partial<Message> = {}): Message =>
    ({ id: 'M1', channel_id: 'D1', author_id: 'U2', kind: 'normal', content: 'hi', reply_to_id: null, created_at: '2026-11-18T12:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null, quote: null, qoty_card: null, clip: null, embeds: [], ...over }) as unknown as Message
  const show = async (m: Message) => {
    invoke.mockImplementation(async (cmd: string) => (cmd === 'server_origin' ? ORIGIN : cmd === 'invite_preview' ? preview() : undefined))
    cmp = mount(MessageItem, {
      target: document.body,
      props: { message: m, serverId: null, name: 'sam', nameOf: () => 'sam', replyTo: null, grouped: false, mine: false, onReply: () => {}, onEdit: async () => {}, onDelete: async () => {} },
    })
    flushSync()
    await settle()
  }
  it('a direct invite shows as its card', async () => {
    await show(msg({ content: `https://pulsechat.co.za/i/${CODE}`, server_invite: { code: CODE, server_id: 'S2' } }))
    expect(document.querySelectorAll('[data-server-invite]')).toHaveLength(1)
    expect(document.querySelector('.content')).toBeNull()
  })
  it('an invite link to this server gets a card under the text; other links do not', async () => {
    await show(msg({ content: `come https://pulsechat.co.za/i/${CODE} and https://example.com/i/${CODE}` }))
    expect([...document.querySelectorAll<HTMLElement>('[data-server-invite]')].map((c) => c.dataset.serverInvite)).toEqual([CODE])
    expect(document.querySelector('.content')).not.toBeNull()
  })
})

describe('Server settings', () => {
  const open = async (section: string, owner = 'U1') => {
    world(owner)
    cmp = mount(ServerSettings, { target: document.body, props: { get server() { return app.state.servers[0] }, onClose: vi.fn() } })
    flushSync()
    ;[...document.querySelectorAll<HTMLButtonElement>('nav button')].find((b) => b.textContent === section)!.click()
    flushSync()
    await settle()
  }
  const sections = () => [...document.querySelectorAll<HTMLButtonElement>('nav button')].map((b) => b.textContent)

  it('Invites lists them, follows InvitesChanged, and Revoke revokes', async () => {
    invoke.mockImplementation(async (cmd: string) => (cmd === 'list_server_invites' ? [invite(), invite({ code: 'zzzzzzzzzz', created_by: 'U2', uses: 1, state: 'used_up' })] : undefined))
    await open('Invites')
    const rows = () => [...document.querySelectorAll<HTMLElement>('[data-invite]')]
    expect(rows().map((r) => r.dataset.invite)).toEqual([CODE, 'zzzzzzzzzz'])
    expect(rows()[0].textContent).toContain('0 / 1')
    expect(rows()[1].textContent).toContain('Used')
    expect(rows()[1].querySelector('button')).toBeNull()
    expect(calls('list_server_invites')).toHaveLength(1)
    app.state = applyEvent(app.state, { t: 'InvitesChanged', d: { user_id: 'U1', server_id: 'S2' } }, 0)
    flushSync()
    await settle()
    expect(calls('list_server_invites')).toHaveLength(1)
    app.state = applyEvent(app.state, { t: 'InvitesChanged', d: { user_id: 'U1', server_id: 'S1' } }, 0)
    flushSync()
    await settle()
    expect(calls('list_server_invites')).toHaveLength(2)
    rows()[0].querySelector('button')!.click()
    await settle()
    expect(calls('revoke_invite')).toEqual([{ code: CODE }])
  })

  it('Bans is the owner\'s: lists them, follows BansChanged, Unban on the right-click menu', async () => {
    let bans = [{ user_id: 'U4', banned_by: 'U1', banned_at: '2026-11-15T08:00:00.000Z' }]
    invoke.mockImplementation(async (cmd: string) => (cmd === 'list_server_bans' ? bans : undefined))
    await open('Bans')
    const rows = () => [...document.querySelectorAll<HTMLElement>('[data-banned]')]
    expect(rows().map((r) => r.dataset.banned)).toEqual(['U4'])
    bans = []
    app.state = applyEvent(app.state, { t: 'BansChanged', d: { user_id: 'U1', server_id: 'S1' } }, 0)
    flushSync()
    await settle()
    expect(rows()).toHaveLength(0)
    bans = [{ user_id: 'U4', banned_by: 'U1', banned_at: '2026-11-15T08:00:00.000Z' }]
    app.state = applyEvent(app.state, { t: 'BansChanged', d: { user_id: 'U1', server_id: 'S1' } }, 0)
    flushSync()
    await settle()
    rows()[0].dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 5, clientY: 5 }))
    flushSync()
    document.querySelector<HTMLElement>('[role=menu] [data-item="unban"]')!.click()
    await settle()
    expect(calls('set_server_ban')).toEqual([{ serverId: 'S1', userId: 'U4', on: false }])
    expect(rows()).toHaveLength(0)
  })

  it('a member sees Invites but not Bans', async () => {
    invoke.mockImplementation(async (cmd: string) => (cmd === 'list_server_invites' ? [] : undefined))
    await open('Invites', 'U2')
    expect(sections()).toContain('Invites')
    expect(sections()).not.toContain('Bans')
  })
})
