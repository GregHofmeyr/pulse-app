// @vitest-environment jsdom
// Hide what I'm playing per person, and Invite to game (#436): the person menu's items (Hide / Show what I'm playing,
// and Invite to game… while you're in a game), Settings › Privacy › Game activity's list with + Add person and Remove,
// the invite card (Join game asks the server where, then opens Steam; ended: greyed out, "No longer in this game";
// GameInviteChanged puts it in place), the DM composer's button, and the notification.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fakeMatchMedia, FakeIntersectionObserver } from '../lib/testMotion'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))
vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver)
vi.stubGlobal('matchMedia', fakeMatchMedia(false).matchMedia)

import MemberMenu from './MemberMenu.svelte'
import HiddenGameList from './HiddenGameList.svelte'
import GameInviteCard from './GameInviteCard.svelte'
import { app, resetState, ui } from '../lib/store.svelte'
import { applyEvent, emptyState } from '../lib/state'
import { notifyMessage } from '../lib/notify'
import type { AlertInput } from '../lib/conversations'
import type { Channel } from '../lib/protocol/Channel'
import type { GameInvite } from '../lib/protocol/GameInvite'
import type { Message } from '../lib/protocol/Message'

const user = (id: string, name: string) => ({ id, username: name, display_name: name, avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } })
const alex = user('U1', 'alex')
const sam = user('U2', 'sam')
const kim = user('U3', 'kim')
const playing = { name: 'Space Game', steam_app_id: 570, started_at: null, art: null, join: null }
const LOBBY = { kind: 'lobby', app_id: 570, lobby_id: '109775240917155841', steam_id: '76500000000000001' }

const invite = (over: Partial<GameInvite> = {}): GameInvite => ({ name: 'Space Game', steam_app_id: 570, art: null, joinable: true, ended: false, ...over })
const message = (author = 'U2', card: GameInvite = invite()): Message =>
  ({ id: 'M1', channel_id: 'D1', author_id: author, kind: 'normal', content: '🎮 Invite to play Space Game', reply_to_id: null, created_at: '2026-11-17T12:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null, quote: null, qoty_card: null, clip: null, embeds: [], game_invite: card }) as unknown as Message
const nameOf = (id: string | null) => ({ U1: 'alex', U2: 'sam', U3: 'kim' } as Record<string, string>)[id ?? ''] ?? '?'

let cmp: ReturnType<typeof mount> | undefined
beforeEach(() => {
  resetState()
  invoke.mockReset()
  invoke.mockResolvedValue(undefined)
  app.state = {
    ...emptyState(), me: alex,
    people: {
      U1: { user: alex, online: true, last_seen_at: null, status: 'online' },
      U2: { user: sam, online: true, last_seen_at: null, status: 'online' },
      U3: { user: kim, online: true, last_seen_at: null, status: 'online' },
    },
    members: { S1: [{ user: alex, nickname: null }, { user: sam, nickname: null }, { user: kim, nickname: null }] },
  } as never
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
})

const inAGame = () => {
  app.state = { ...app.state, people: { ...app.state.people, U1: { ...app.state.people.U1, playing } } } as never
}

describe('the person menu', () => {
  const open = (userId = 'U2') => {
    const opener = document.createElement('button')
    document.body.append(opener)
    cmp = mount(MemberMenu, { target: document.body, props: { serverId: 'S1', userId, x: 10, y: 10, opener, onClose: () => {} } })
    flushSync()
  }
  const item = (name: string) => document.querySelector<HTMLElement>(`[role=menu] [data-item="${name}"]`)

  it('Hide what I\'m playing asks the server and flips to Show from its answer', async () => {
    invoke.mockImplementation(async (cmd: string) => (cmd === 'set_activity_hidden' ? ['U2'] : undefined))
    open()
    expect(item('hide-game')!.textContent?.trim()).toBe("Hide what I'm playing")
    item('hide-game')!.click()
    await vi.waitFor(() => expect(app.state.activityHidden).toEqual(['U2']))
    expect(invoke).toHaveBeenCalledWith('set_activity_hidden', { userId: 'U2', on: true })
  })

  it('hidden: Show what I\'m playing turns it back', async () => {
    app.state = { ...app.state, activityHidden: ['U2'] }
    invoke.mockImplementation(async (cmd: string) => (cmd === 'set_activity_hidden' ? [] : undefined))
    open()
    expect(item('hide-game')!.textContent?.trim()).toBe("Show what I'm playing")
    item('hide-game')!.click()
    await vi.waitFor(() => expect(app.state.activityHidden).toEqual([]))
    expect(invoke).toHaveBeenCalledWith('set_activity_hidden', { userId: 'U2', on: false })
  })

  it('Invite to game… only while you are in a game: it opens your DM with them and sends the invite there', async () => {
    open()
    expect(item('invite-game')).toBeNull()
    unmount(cmp!)
    document.body.innerHTML = ''
    inAGame()
    const dm = { id: 'D1', server_id: null, kind: 'dm', name: null, position: 0 }
    invoke.mockImplementation(async (cmd: string) => (cmd === 'create_dm' ? dm : cmd === 'send_game_invite' ? message('U1') : undefined))
    open()
    expect(item('invite-game')!.textContent).toBe('Invite to game…')
    item('invite-game')!.click()
    await vi.waitFor(() => expect(ui.goTo).toBe('D1'))
    expect(invoke).toHaveBeenCalledWith('create_dm', { userIds: ['U2'] })
    expect(invoke).toHaveBeenCalledWith('send_game_invite', { channelId: 'D1' })
  })

  it('neither for yourself', () => {
    inAGame()
    open('U1')
    expect(item('hide-game')).toBeNull()
    expect(item('invite-game')).toBeNull()
  })
})

describe('Settings › Privacy › Game activity', () => {
  const rows = () => [...document.querySelectorAll<HTMLElement>('[data-hidden-games] li')]
  const show = () => {
    cmp = mount(HiddenGameList, { target: document.body })
    flushSync()
  }

  it('lists them by name; Remove shows them again; the list follows the event', async () => {
    app.state = { ...app.state, activityHidden: ['U3', 'U2'] }
    invoke.mockImplementation(async (cmd: string) => (cmd === 'set_activity_hidden' ? ['U2'] : undefined))
    show()
    expect(rows().map((r) => r.querySelector('.who')!.textContent)).toEqual(['kim', 'sam'])
    rows()[0].querySelector<HTMLButtonElement>('button')!.click()
    expect(invoke).toHaveBeenCalledWith('set_activity_hidden', { userId: 'U3', on: false })
    await vi.waitFor(() => expect(rows()).toHaveLength(1))
    app.state = applyEvent(app.state, { t: 'ActivityHiddenChanged', d: { user_id: 'U1', list: ['U3'] } }, 0)
    flushSync()
    expect(rows().map((r) => r.querySelector('.who')!.textContent)).toEqual(['kim'])
  })

  it('+ Add person offers those not on it; picking hides your games from them', async () => {
    app.state = { ...app.state, activityHidden: ['U3'] }
    invoke.mockImplementation(async (cmd: string) => (cmd === 'set_activity_hidden' ? ['U3', 'U2'] : undefined))
    show()
    ;[...document.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.includes('Add person'))!.click()
    flushSync()
    const dialog = document.querySelector<HTMLElement>('.overlay')!
    expect(dialog.textContent).toContain('sam')
    expect(dialog.textContent).not.toContain('kim')
  })

  it('empty: says how to add someone', () => {
    show()
    expect(rows()).toEqual([])
    expect(document.querySelector('[data-hidden-games] .empty')!.textContent).toContain("Hide what I'm playing")
  })
})

describe('the invite card', () => {
  const show = (m: Message) => {
    cmp = mount(GameInviteCard, { target: document.body, props: { message: m, nameOf } })
    flushSync()
  }
  const join = () => document.querySelector<HTMLButtonElement>('[data-game-invite] .join')

  it("who's inviting, the game, and Join game: asks the server where, then opens Steam there", async () => {
    invoke.mockImplementation(async (cmd: string) => (cmd === 'game_invite_join' ? LOBBY : undefined))
    show(message())
    const card = document.querySelector<HTMLElement>('[data-game-invite]')!
    expect(card.textContent).toContain('sam invited you to play')
    expect(card.textContent).toContain('Space Game')
    expect(join()!.textContent).toContain('Join game')
    join()!.click()
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('join_steam_game', { join: LOBBY }))
    expect(invoke).toHaveBeenCalledWith('game_invite_join', { messageId: 'M1' })
  })

  it('nothing to join: no button; your own: no Join', () => {
    show(message('U2', invite({ joinable: false })))
    expect(join()).toBeNull()
    unmount(cmp!)
    document.body.innerHTML = ''
    show(message('U1'))
    expect(document.querySelector('[data-game-invite]')!.textContent).toContain('You invited them to play')
    expect(join()).toBeNull()
  })

  it('ended: greyed out, "No longer in this game", and GameInviteChanged puts it in place', () => {
    show(message('U2', invite({ joinable: false, ended: true })))
    expect(join()!.disabled).toBe(true)
    expect(join()!.textContent).toBe('No longer in this game')
    // the reducer: a loaded invite takes the new card
    app.state = { ...app.state, messages: { D1: [message()] } } as never
    app.state = applyEvent(app.state, { t: 'GameInviteChanged', d: { channel_id: 'D1', message_id: 'M1', invite: invite({ joinable: false, ended: true }) } }, 0)
    expect(app.state.messages.D1[0].game_invite).toEqual(invite({ joinable: false, ended: true }))
  })
})

describe('the notification', () => {
  const dm = { id: 'D1', server_id: null, kind: 'dm', name: null, position: 0 } as unknown as Channel
  const v = (m: Message): AlertInput => ({ message: m, me: 'U1', channel: dm, open: false, openThread: null, focused: false, muted: false, level: 'all' })

  it('"sam invited you to play Space Game"; with message text hidden, the game is too', () => {
    const n = notifyMessage(app.state, v(message()), { enabled: true, showText: true })!
    expect(n.title).toBe('sam invited you to play Space Game')
    const quiet = notifyMessage(app.state, v(message()), { enabled: true, showText: false })!
    expect(quiet.title).toBe('sam')
    expect(quiet.body).toBe('Invited you to play a game')
  })
})
