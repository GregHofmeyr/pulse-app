// @vitest-environment jsdom
import { flushSync, mount, tick, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...a), convertFileSrc: (s: string) => s }))
/** The core's events, by name: tests emit them as the Rust side would. */
const handlers = vi.hoisted(() => new Map<string, (e: { payload: unknown }) => void>())
/** Events whose listen is refused. */
const listenFails = vi.hoisted(() => new Set<string>())
vi.mock('@tauri-apps/api/event', () => ({
  listen: async (name: string, fn: (e: { payload: unknown }) => void) => {
    if (listenFails.has(name)) throw new Error(`can't listen to ${name}`)
    handlers.set(name, fn)
    return () => handlers.delete(name)
  },
}))
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ onFocusChanged: async () => () => {}, isFocused: async () => true, minimize() {}, toggleMaximize() {}, close() {}, startDragging() {} }),
}))
const room = vi.hoisted(() => ({
  connected: true,
  onLost: () => {},
  disconnect: async () => {},
  watch: async () => 0,
  unwatch: () => {},
  setLayer: vi.fn(),
  remoteStats: async () => undefined,
}))
vi.mock('../components/GoLiveDialog.svelte', () => ({ default: () => {} }))
vi.mock('../lib/stream/room', () => ({ StreamRoom: { connect: async () => room, pick: async () => null }, STREAM_UNAVAILABLE: 'unavailable' }))

import Shell from './Shell.svelte'
import LivePopover from '../components/LivePopover.svelte'
import { app, startListening } from '../lib/store.svelte'
import { applyEvent, applyReady, emptyState } from '../lib/state'
import { CONN_PILL_DELAY_MS } from '../lib/conn'
import { startVoiceListening, voice } from '../lib/voice.svelte'
import { stream, streamApi } from '../lib/stream/stream.svelte'
import { closeGoLive, goLiveUi } from '../lib/stream/golive.svelte'
import { callView, joinAndWatch, openFull, showToast, watchStream } from '../lib/stream/viewer.svelte'
import type { Ready } from '../lib/protocol/Ready'
import Waiting from './Waiting.svelte'
import { reloadColumns } from '../lib/columns.svelte'
import { reloadVoiceChat, voiceChat } from '../lib/voiceChat.svelte'
import { testMe, testReady } from '../lib/testReady'
import { ui } from '../lib/store.svelte'
import { editProfile, openCardSettings, profileUi } from '../lib/profileCard.svelte'
import { reloadPatchNotesSeen } from '../lib/patchNotesSeen.svelte'
import { patchNotes, previousPatchNotes } from '../patchNotes'
import { startUpdateListening, updates } from '../lib/updates.svelte'

let cmp: ReturnType<typeof mount> | undefined
let offs: (() => void)[] = []
beforeEach(() => {
  invoke.mockReset()
  // a text channel opening asks for its history
  invoke.mockImplementation(async (cmd: string) => (cmd === 'list_messages' ? [] : undefined))
  // jsdom has no scrolling
  Element.prototype.scrollTo ??= () => {}
})
afterEach(async () => {
  if (cmp) unmount(cmp)
  cmp = undefined
  offs.forEach((off) => off())
  offs = []
  callView.full = null
  callView.goTo = null
  callView.joinThenWatch = null
  await streamApi.unwatchAll()
  // a stream's toast ("sam stopped streaming") would be the next test's first .notice
  stream.notice = ''
  stream.watchError = ''
  document.body.innerHTML = ''
  vi.restoreAllMocks()
})

describe('Shell', () => {
  it('holds animation still while live, and lets go when live ends or the shell goes', async () => {
    const { motion } = await import('../lib/animate.svelte')
    app.state = emptyState()
    cmp = mount(Shell, { target: document.body, props: { user: { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }, onLogout: () => {} } })
    flushSync()
    const live = { channelId: 'V1', res: 720, fps: 60, audio: false, audioMuted: false, label: 'Game', encoderWarning: false, allowGhosts: true } as typeof stream.live
    stream.live = live
    flushSync()
    expect(motion.streaming).toBe(true)
    expect(document.body.classList.contains('streaming')).toBe(true)
    stream.live = null
    flushSync()
    expect(motion.streaming).toBe(false)
    stream.live = live
    flushSync()
    unmount(cmp)
    cmp = undefined
    expect(motion.streaming).toBe(false)
    expect(document.body.classList.contains('streaming')).toBe(false)
    stream.live = null
  })

  it('Esc closes the server menu and puts focus back on the server name', async () => {
    app.state = { ...emptyState(), servers: [{ id: 'S1', name: 'Alpha', icon_hash: null }] }
    cmp = mount(Shell, { target: document.body, props: { user: { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }, onLogout: () => {} } })
    flushSync()
    ;[...document.querySelectorAll<HTMLButtonElement>('.tab')].find((t) => t.textContent!.includes('Alpha'))!.click()
    flushSync()
    const toggle = document.querySelector<HTMLButtonElement>('.head-btn')!
    toggle.click()
    flushSync()
    await new Promise((r) => setTimeout(r, 0)) // clickOutside listens from the next tick
    ;[...document.querySelectorAll<HTMLButtonElement>('[role=menuitem]')][0].focus()
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
    flushSync()
    expect(document.querySelector('.head-menu')).toBeNull()
    expect(document.activeElement).toBe(toggle)
  })

  it('server settings and the server menu close when you switch server', () => {
    app.state = { ...emptyState(), servers: [{ id: 'S1', name: 'Alpha', icon_hash: null }, { id: 'S2', name: 'Beta', icon_hash: null }] }
    cmp = mount(Shell, { target: document.body, props: { user: { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }, onLogout: () => {} } })
    flushSync()
    const tab = (name: string) => [...document.querySelectorAll<HTMLButtonElement>('.tab')].find((t) => t.textContent!.includes(name))!
    tab('Alpha').click()
    flushSync()
    document.querySelector<HTMLButtonElement>('.head-btn[data-menu-toggle]')!.click()
    flushSync()
    ;[...document.querySelectorAll<HTMLButtonElement>('[role=menuitem]')].find((b) => b.textContent === 'Server settings')!.click()
    flushSync()
    expect(document.querySelector('[role=dialog][aria-label="Server settings"]')).not.toBeNull()
    tab('Beta').click()
    flushSync()
    expect(document.querySelector('[role=dialog][aria-label="Server settings"]')).toBeNull()
    expect(document.querySelector('[role=menu]')).toBeNull()
  })

  it('the top bar says why a refused connection is retrying', () => {
    app.state = { ...emptyState(), conn: 'too_many_devices' }
    cmp = mount(Shell, { target: document.body, props: { user: { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }, onLogout: () => {} } })
    flushSync()
    expect(document.querySelector('.conn')!.textContent).toBe('Too many devices connected — close Pulse on another device')
    app.connAt = Date.now()
    app.state = { ...app.state, conn: { rate_limited: 4 } }
    flushSync()
    expect(document.querySelector('.conn')!.textContent).toBe('Too many attempts, retrying in 4s…')
  })

  it('a quick reconnect doesn\'t flash the pill: it shows once the connection has been down a moment', () => {
    vi.useFakeTimers()
    try {
      app.state = { ...emptyState(), conn: 'connected' }
      cmp = mount(Shell, { target: document.body, props: { user: { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }, onLogout: () => {} } })
      flushSync()
      app.state = { ...app.state, conn: 'connecting' }
      flushSync()
      expect(document.querySelector('.conn')).toBeNull()
      vi.advanceTimersByTime(CONN_PILL_DELAY_MS - 100)
      flushSync()
      // back before the delay: it never showed
      app.state = { ...app.state, conn: 'connected' }
      flushSync()
      vi.advanceTimersByTime(1000)
      flushSync()
      expect(document.querySelector('.conn')).toBeNull()
      // down for longer: it shows, and goes once connected
      app.state = { ...app.state, conn: 'connecting' }
      flushSync()
      vi.advanceTimersByTime(CONN_PILL_DELAY_MS)
      flushSync()
      expect(document.querySelector('.conn')!.textContent).toBe('Connecting…')
      app.state = { ...app.state, conn: 'connected' }
      flushSync()
      expect(document.querySelector('.conn')).toBeNull()
    } finally {
      vi.useRealTimers()
    }
  })

  it('the retry countdown ticks down and is replaced when the next attempt starts', () => {
    vi.useFakeTimers()
    try {
      vi.setSystemTime(100_000)
      app.connAt = Date.now()
      app.state = { ...emptyState(), conn: { rate_limited: 3 } }
      cmp = mount(Shell, { target: document.body, props: { user: { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }, onLogout: () => {} } })
      flushSync()
      expect(document.querySelector('.conn')!.textContent).toBe('Too many attempts, retrying in 3s…')
      vi.advanceTimersByTime(1000)
      flushSync()
      expect(document.querySelector('.conn')!.textContent).toBe('Too many attempts, retrying in 2s…')
      vi.advanceTimersByTime(2000)
      flushSync()
      expect(document.querySelector('.conn')!.textContent).toBe('Connecting…')
    } finally {
      vi.useRealTimers()
    }
  })

  it('a refused startup shows the message on the waiting screen', () => {
    app.connAt = Date.now()
    app.state = { ...emptyState(), conn: 'too_many_devices' }
    cmp = mount(Waiting, { target: document.body, props: { onSignOut: () => {} } })
    flushSync()
    expect(document.querySelector('[role=status]')!.textContent).toContain('Too many devices connected')
  })
})

describe('account menu', () => {
  it('shows your profile picture and styled name, and follows your profile live', () => {
    const atLogin = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
    const pic = 'a'.repeat(64)
    app.state = { ...emptyState(), me: { ...atLogin, avatar_hash: pic, style: { font: null, colors: ['#ff8800'] } } }
    cmp = mount(Shell, { target: document.body, props: { user: atLogin, onLogout: () => {} } })
    flushSync()
    const btn = () => document.querySelector<HTMLElement>('.me')!
    expect(btn().querySelector('img')?.getAttribute('src')).toContain(pic)
    expect(btn().querySelector<HTMLElement>('.name')?.getAttribute('style')).toContain('color:')
    expect(btn().textContent).toContain('alex')
    // a ProfileUpdated for you swaps app.state.me: the panel follows without a restart
    const next = 'b'.repeat(64)
    app.state = { ...app.state, me: { ...app.state.me!, avatar_hash: next } }
    flushSync()
    expect(btn().querySelector('img')?.getAttribute('src')).toContain(next)
  })

  it('one click on your avatar or name opens your own profile card, and nothing else (#168)', () => {
    const me = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
    app.state = { ...emptyState(), me }
    cmp = mount(Shell, { target: document.body, props: { user: me, onLogout: () => {} } })
    flushSync()
    for (const part of ['.me-avatar', '.who']) {
      const target = document.querySelector<HTMLElement>(`.me ${part}`)!
      const button = target.closest<HTMLButtonElement>('button')!
      button.click()
      flushSync()
      expect(profileUi.card, part).toMatchObject({ userId: 'U1', serverId: null, opener: button })
      expect(button.getAttribute('aria-expanded'), part).toBe('true')
      // no status menu and no account menu: those live on the card and in Settings now
      expect(document.querySelector('[role=menu]'), part).toBeNull()
      // the same click again closes it
      button.click()
      flushSync()
      expect(profileUi.card, part).toBeNull()
    }
    // avatar and name are one control
    expect(document.querySelector('.me .me-avatar')!.closest('button')).toBe(document.querySelector('.me .who')!.closest('button'))
    // the cog stays beside it
    expect(document.querySelector('.me button[aria-label="Settings"]')).not.toBeNull()
    // the new-badges dot stays on the avatar
    expect(document.querySelector('.me .new-dot')).toBeNull()
    app.state = { ...app.state, profiles: { U1: { badges: [{ key: 'first_words', seen: false }] } } } as never
    flushSync()
    expect(document.querySelector('.me .me-avatar .new-dot')).not.toBeNull()
    expect(document.querySelector('.me .me-avatar')!.closest('button')!.getAttribute('aria-label')).toBe('Your profile (new badges)')
  })

  it('Settings on your card opens Settings; Log out at the bottom of Settings logs out (#168)', async () => {
    const me = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
    app.state = { ...emptyState(), me }
    const onLogout = vi.fn()
    cmp = mount(Shell, { target: document.body, props: { user: me, onLogout } })
    flushSync()
    openCardSettings()
    flushSync()
    expect(profileUi.settings).toBe(false)
    const dialog = document.querySelector('[role=dialog][aria-label="Settings"]')!
    expect(dialog).not.toBeNull()
    const out = [...dialog.querySelectorAll<HTMLButtonElement>('nav button')].at(-1)!
    expect(out.textContent?.trim()).toBe('Log out')
    out.click()
    await vi.waitFor(() => expect(onLogout).toHaveBeenCalledTimes(1))
    expect(invoke.mock.calls.some(([cmd]) => cmd === 'logout')).toBe(true)
  })

  // #155: Report a bug is a bug icon in the title bar, just left of Minimise, not an account menu item. #218: it takes ideas too.
  it('Report a bug is the title bar icon left of Minimise, and it opens the report dialog', () => {
    const me = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
    app.state = { ...emptyState(), me }
    cmp = mount(Shell, { target: document.body, props: { user: me, onLogout: () => {} } })
    flushSync()
    const bar = [...document.querySelectorAll<HTMLButtonElement>('header > button')]
    const bug = bar.find((b) => b.getAttribute('aria-label') === 'Report a bug or suggest an idea')!
    expect(bug).toBeDefined()
    expect(bug.textContent?.trim()).toBe('') // icon only: the tooltip comes from its aria-label
    expect(bug.querySelector('svg')).not.toBeNull()
    expect(bar[bar.indexOf(bug) + 1].getAttribute('aria-label')).toBe('Minimise')
    bug.click()
    flushSync()
    expect(document.querySelector('[role=dialog][aria-label="Report a bug"]')).not.toBeNull()
  })

  // #82: a newer version while running: "Restart to update" left of Patch notes, never during a call.
  describe('Restart to update', () => {
    const me = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
    const open = async () => {
      app.state = { ...emptyState(), me }
      offs.push(await startUpdateListening())
      cmp = mount(Shell, { target: document.body, props: { user: me, onLogout: () => {} } })
      flushSync()
    }
    const button = () => [...document.querySelectorAll<HTMLButtonElement>('header > button')].find((b) => b.textContent?.trim() === 'Restart to update')
    const offered = () => {
      handlers.get('pulse://update')!({ payload: { version: '0.2.1' } })
      flushSync()
    }
    // out of any call and with no offer, before (a test elsewhere may have left a call up) and after
    const reset = () => {
      updates.offer = null
      updates.installing = false
      updates.error = null
      voice.channelId = null
      voice.joining = null
      voice.stage = { stage: 'disconnected', attempt: null }
    }
    beforeEach(reset)
    afterEach(reset)

    // #233: the core also refuses while a call is reconnecting (or being joined), so the button says so too
    it('a call that is reconnecting or being joined disables it too', async () => {
      await open()
      offered()
      voice.stage = { stage: 'reconnecting', attempt: 2 }
      flushSync()
      expect(button()!.getAttribute('aria-disabled')).toBe('true')
      expect(button()!.getAttribute('data-tip')).toBe('Finish your call first')
      voice.stage = { stage: 'disconnected', attempt: null }
      voice.joining = 'c2'
      flushSync()
      expect(button()!.getAttribute('aria-disabled')).toBe('true')
      voice.joining = null
      flushSync()
      expect(button()!.getAttribute('aria-disabled')).not.toBe('true')
    })

    it("the core's refusal shows on the button instead of vanishing (#233)", async () => {
      invoke.mockImplementation(async (cmd: string) => {
        if (cmd === 'update_install') throw 'Finish your call first'
        return cmd === 'list_messages' ? [] : undefined
      })
      await open()
      offered()
      button()!.click()
      await vi.waitFor(() => expect(updates.installing).toBe(false))
      flushSync()
      expect(button()!.getAttribute('data-tip')).toBe("Couldn't update: Finish your call first")
    })

    it('an offer the server withdrew goes away (#233)', async () => {
      await open()
      offered()
      expect(button()).toBeDefined()
      handlers.get('pulse://update')!({ payload: null })
      flushSync()
      expect(button()).toBeUndefined()
    })

    it('no offer, no button; an offer shows it just left of Patch notes', async () => {
      await open()
      expect(button()).toBeUndefined()
      offered()
      const bar = [...document.querySelectorAll<HTMLButtonElement>('header > button')]
      const b = button()!
      expect(b).toBeDefined()
      expect(b.classList.contains('win')).toBe(true)
      expect(bar[bar.indexOf(b) + 1].getAttribute('aria-label')).toBe('Patch notes')
      expect(b.getAttribute('aria-disabled')).not.toBe('true')
    })

    it('in a call the button is disabled, with the tooltip, and a click does nothing; leaving the call enables it', async () => {
      await open()
      offered()
      voice.channelId = 'c1'
      flushSync()
      const b = button()!
      expect(b.getAttribute('aria-disabled')).toBe('true')
      expect(b.hasAttribute('disabled')).toBe(false) // hoverable, so the tooltip shows
      expect(b.getAttribute('data-tip')).toBe('Finish your call first')
      b.click()
      flushSync()
      expect(invoke).not.toHaveBeenCalledWith('update_install', expect.anything())
      expect(document.querySelector('.updating')).toBeNull()
      voice.channelId = null
      flushSync()
      expect(button()!.getAttribute('aria-disabled')).not.toBe('true')
      expect(button()!.hasAttribute('data-tip')).toBe(false)
    })

    it('click installs with restart; a failure goes back to the shell with the button still there, and is logged', async () => {
      let fail: (e: unknown) => void = () => {}
      invoke.mockImplementation(async (cmd: string) => {
        if (cmd === 'update_install') return new Promise((_, reject) => (fail = reject))
        return cmd === 'list_messages' ? [] : undefined
      })
      await open()
      offered()
      button()!.click()
      flushSync()
      expect(invoke).toHaveBeenCalledWith('update_install', { restart: true })
      expect(updates.installing).toBe(true)
      fail('no network')
      await vi.waitFor(() => expect(updates.installing).toBe(false))
      flushSync()
      expect(button()).toBeDefined()
      expect(document.querySelector('header')).not.toBeNull()
      expect(invoke).toHaveBeenCalledWith('ui_log', expect.objectContaining({ line: expect.stringContaining('no network') }))
    })
  })

  // #207: Patch notes sit just left of Report a bug, with a dot until this release's notes are opened once here.
  describe('Patch notes', () => {
    const me = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
    const open = () => {
      app.state = { ...emptyState(), me }
      cmp = mount(Shell, { target: document.body, props: { user: me, onLogout: () => {} } })
      flushSync()
    }
    const icon = () => [...document.querySelectorAll<HTMLButtonElement>('header > button')].find((b) => b.getAttribute('aria-label') === 'Patch notes')!
    afterEach(() => {
      localStorage.clear()
      reloadPatchNotesSeen()
    })

    it('is a title bar icon just left of Report a bug, and it opens the notes', () => {
      open()
      const bar = [...document.querySelectorAll<HTMLButtonElement>('header > button')]
      const b = icon()
      expect(b).toBeDefined()
      expect(b.textContent?.trim()).toBe('') // icon only: the tooltip comes from its aria-label
      expect(b.querySelector('svg')).not.toBeNull()
      expect(bar[bar.indexOf(b) + 1].getAttribute('aria-label')).toBe('Report a bug or suggest an idea')
      b.click()
      flushSync()
      expect(document.querySelector('[role=dialog][aria-label="Patch notes"]')).not.toBeNull()
      expect(document.body.textContent).toContain(patchNotes.date)
      // #235: the release before, collapsed beneath
      expect(document.querySelector('details.previous summary')?.textContent?.trim()).toBe(`Previous update (${previousPatchNotes!.version})`)
    })

    it('shows a dot until the notes are opened, and opening clears it for good', () => {
      open()
      expect(icon().querySelector('.new-dot')).not.toBeNull()
      icon().click()
      flushSync()
      expect(icon().querySelector('.new-dot')).toBeNull()
      expect(localStorage.getItem('pulse.patchNotesSeen')).toBe(patchNotes.id)
      unmount(cmp!)
      reloadPatchNotesSeen()
      open()
      expect(icon().querySelector('.new-dot')).toBeNull()
    })

    it('the dot comes back for a new release (a different id)', () => {
      localStorage.setItem('pulse.patchNotesSeen', 'an-older-release')
      reloadPatchNotesSeen()
      open()
      expect(icon().querySelector('.new-dot')).not.toBeNull()
    })
  })

  it('your avatar shows your status dot (Invisible as the grey offline dot, #171); the line under your name says it (#45)', () => {
    const me = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
    app.state = { ...emptyState(), me, conn: 'connected', myStatus: { status: 'invisible', dnd_until: null } }
    cmp = mount(Shell, { target: document.body, props: { user: me, onLogout: () => {} } })
    flushSync()
    const dot = document.querySelector('.me .me-avatar .pres')!
    expect(dot.classList.contains('invisible')).toBe(true)
    expect(document.querySelector('.me .who small')!.textContent).toBe('Invisible')
    app.state = { ...app.state, myStatus: { status: 'dnd', dnd_until: null } }
    flushSync()
    expect(document.querySelector('.me .me-avatar .pres')!.classList.contains('dnd')).toBe(true)
    expect(document.querySelector('.me .who small')!.textContent).toBe('Do Not Disturb')
    // the new-badges dot stays on the avatar
    expect(document.querySelector('.me .me-avatar')!.closest('button')!.getAttribute('aria-label')).toBe('Your profile')
  })

  it('Edit profile on your card opens Settings on Profile', () => {
    const me = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
    app.state = { ...emptyState(), me }
    cmp = mount(Shell, { target: document.body, props: { user: me, onLogout: () => {} } })
    flushSync()
    editProfile()
    flushSync()
    expect(document.querySelector('[aria-current=page]')?.textContent).toContain('Profile')
    expect(profileUi.edit).toBe(false)
  })

  it('esc_closes_the_server_mute_menu', async () => {
    app.state = { ...emptyState(), servers: [{ id: 'S1', name: 'Alpha', icon_hash: null }] }
    cmp = mount(Shell, { target: document.body, props: { user: { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }, onLogout: () => {} } })
    flushSync()
    const tab = [...document.querySelectorAll<HTMLButtonElement>('.tab')].find((t) => t.textContent!.includes('Alpha'))!
    tab.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 10, clientY: 10 }))
    flushSync()
    expect(document.querySelector('[role=menu][aria-label="Actions for Alpha"]')).not.toBeNull()
    await new Promise((r) => setTimeout(r, 0))
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', cancelable: true }))
    flushSync()
    expect(document.querySelector('[role=menu][aria-label="Actions for Alpha"]')).toBeNull()
  })

  it('the_server_tab_menu_opens_that_servers_settings (#161)', async () => {
    app.state = { ...emptyState(), servers: [{ id: 'S1', name: 'Alpha', icon_hash: null }, { id: 'S2', name: 'Beta', icon_hash: null }] }
    cmp = mount(Shell, { target: document.body, props: { user: { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }, onLogout: () => {} } })
    flushSync()
    const tab = [...document.querySelectorAll<HTMLButtonElement>('.tab')].find((t) => t.textContent!.includes('Beta'))!
    tab.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 10, clientY: 10 }))
    flushSync()
    const menu = document.querySelector<HTMLElement>('[role=menu][aria-label="Actions for Beta"]')!
    expect([...menu.querySelectorAll(':scope > [role=menuitem]')].map((b) => b.textContent?.trim())).toEqual(['Server settings', 'Mute server', 'Notifications'])
    ;[...menu.querySelectorAll<HTMLButtonElement>('[role=menuitem]')].find((b) => b.textContent?.trim() === 'Server settings')!.click()
    flushSync()
    expect(document.querySelector('[role=menu][aria-label="Actions for Beta"]')).toBeNull()
    expect(document.querySelector('[role=dialog][aria-label="Server settings"]')).not.toBeNull()
    expect(document.querySelector('[role=dialog][aria-label="Server settings"]')!.textContent).toContain('Beta')
  })

  describe('Create category (#54)', () => {
    const alex = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
    function world() {
      app.state = {
        ...emptyState(),
        me: alex,
        servers: [{ id: 'S1', name: 'Alpha', icon_hash: null }, { id: 'S2', name: 'Beta', icon_hash: null }],
        members: { S1: [{ user: alex, nickname: null }], S2: [{ user: alex, nickname: null }] },
        layouts: { S1: { revision: 1, categories: [] }, S2: { revision: 1, categories: [] } },
      }
      cmp = mount(Shell, { target: document.body, props: { user: alex, onLogout: () => {} } })
      flushSync()
    }
    const tab = (name: string) => [...document.querySelectorAll<HTMLButtonElement>('.tab')].find((t) => t.textContent!.includes(name))!
    const field = () => document.querySelector<HTMLInputElement>('.list input[aria-label="New category name"]')

    it('the sidebar header menu opens the new category field in the list', () => {
      world()
      tab('Alpha').click()
      flushSync()
      document.querySelector<HTMLButtonElement>('.head-btn')!.click()
      flushSync()
      ;[...document.querySelectorAll<HTMLButtonElement>('.head-menu [role=menuitem]')].find((b) => b.textContent === 'Create category')!.click()
      flushSync()
      expect(document.querySelector('.head-menu')).toBeNull()
      expect(field()).not.toBeNull()
    })

    it("a tab's menu switches to that server and opens the field there", () => {
      world()
      tab('Alpha').click()
      flushSync()
      tab('Beta').dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 10, clientY: 10 }))
      flushSync()
      const menu = document.querySelector<HTMLElement>('[role=menu][aria-label="Actions for Beta"]')!
      ;[...menu.querySelectorAll<HTMLButtonElement>('[role=menuitem]')].find((b) => b.textContent?.trim() === 'Create category')!.click()
      flushSync()
      expect(document.querySelector('.head-btn')!.textContent).toContain('Beta')
      expect(field()).not.toBeNull()
    })
  })
})

describe('watching from anywhere', () => {
  const me = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
  const sam = { id: 'U2', username: 'sam', display_name: 'sam', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
  const flags = { muted: false, deafened: false }
  // voice-only: out of the call, Watch joins first (the ghost path has its own tests)
  const samStream = { channel_id: 'V1', user_id: 'U2', settings: { height: 720, fps: 60, audio: false, allow_ghosts: false }, status: 'live' as const, viewers: [], ghosts: 0, started_at: '2026-10-26T10:00:00Z' }
  const ready: Ready = {
    me,
    servers: [{ id: 'S1', name: 'Main', icon_hash: null }],
    channels: [
      { id: 'T1', server_id: 'S1', kind: 'text', name: 'general', position: 0 },
      { id: 'V1', server_id: 'S1', kind: 'voice', name: 'Lounge', position: 1 },
    ],
    members: [{ server_id: 'S1', members: [{ user: me, nickname: null }, { user: sam, nickname: null }] }],
    dm_members: [], voice: [{ channel_id: 'V1', members: [{ user_id: 'U1', flags }, { user_id: 'U2', flags }] }],
    people: [], read_states: [], mutes: [], hidden: [], latest: [], streams: [samStream], custom_emoji: [], frequent_emoji: [], qoty: [], speaker_links: [], thread_seen: [], games: [], clip_views: [], private_nicknames: [], sounds: [], sound_favourites: [], frequent_sounds: [], server_voice: [], status: { status: 'online', dnd_until: null }, appear_offline_to: [], live_notify: [],
  }
  const SAM = { channelId: 'V1', streamer: 'U2' }
  const q = (sel: string) => document.querySelector<HTMLElement>(sel)
  const button = (text: string) => [...document.querySelectorAll('button')].find((b) => b.textContent?.trim() === text || b.getAttribute('aria-label') === text)
  const settle = async () => {
    for (let i = 0; i < 5; i++) await tick()
    await new Promise((r) => setTimeout(r, 0))
    flushSync()
  }
  const emit = (name: string, payload: unknown) => handlers.get(name)!({ payload })
  const goTo = (id: string) => {
    callView.goTo = id
    flushSync()
  }

  beforeEach(async () => {
    localStorage.clear()
    reloadVoiceChat() // read from storage once: a voice chat left open would take the members list's place
    vi.spyOn(navigator, 'userAgent', 'get').mockReturnValue('Mozilla/5.0 (Windows NT 10.0)')
    vi.spyOn(HTMLMediaElement.prototype, 'play').mockResolvedValue()
    app.state = applyReady(emptyState(), ready)
    voice.channelId = 'V1'
    voice.joining = null
    voice.leaving = null
    voice.joinFailed = null
    voice.connection = 'connected'
    voice.stage = { stage: 'connected', attempt: null }
    stream.live = null
    stream.watching = []
    stream.notice = ''
    stream.watchError = ''
    offs = [await startListening(), await startVoiceListening()]
    cmp = mount(Shell, { target: document.body, props: { user: me, onLogout: () => {} } })
    flushSync()
  })

  const toggle = (name: 'Channels' | 'Members') => document.querySelector<HTMLButtonElement>(`.full-view .panel-toggle[aria-label="${name}"]`)

  it('full_view_hides_both_side_panels_with_top_corner_toggles_to_bring_each_back', async () => {
    const setItem = vi.spyOn(Storage.prototype, 'setItem')
    const side = () => q('aside.side')!
    const members = () => q('.right-panel')!
    expect(side().hidden).toBe(false)
    expect(members().hidden).toBe(false)
    goTo('V1')
    await streamApi.watch('V1', 'U2')
    openFull(SAM)
    await settle()
    expect(q('.full-view')).not.toBeNull()
    expect(side().hidden).toBe(true)
    expect(members().hidden).toBe(true)
    const left = toggle('Channels')!
    const right = toggle('Members')!
    // what each toggles, where it sits: the full view's top bar, the left one first and the right one last
    expect(left.getAttribute('aria-controls')).toBe(side().id)
    expect(right.getAttribute('aria-controls')).toBe(members().id)
    const bar = q('.full-view .bar')!
    expect(bar.firstElementChild).toBe(left)
    expect(bar.lastElementChild).toBe(right)
    // no edge tabs on the stream any more
    expect(q('.edge')).toBeNull()
    for (const t of [left, right]) {
      expect(t.type).toBe('button')
      expect(t.querySelector('svg')).not.toBeNull() // an icon
      expect(t.getAttribute('aria-pressed')).toBe('false')
      expect(t.hasAttribute('aria-expanded')).toBe(false) // pressed says it; both would be read twice
      expect(t.tabIndex).toBe(0)
    }
    expect(left.dataset.tip).toBe('Show channels')
    expect(right.dataset.tip).toBe('Show members')
    left.click()
    flushSync()
    expect(side().hidden).toBe(false)
    expect(members().hidden).toBe(true) // each on its own
    expect(left.getAttribute('aria-pressed')).toBe('true')
    expect(left.dataset.tip).toBe('Hide channels')
    expect(right.getAttribute('aria-pressed')).toBe('false')
    right.click()
    flushSync()
    expect(members().hidden).toBe(false)
    expect(right.getAttribute('aria-pressed')).toBe('true')
    // they take keyboard focus (a native button: Enter and Space click it)
    right.focus()
    expect(document.activeElement).toBe(right)
    right.click()
    flushSync()
    expect(members().hidden).toBe(true)
    expect(right.getAttribute('aria-pressed')).toBe('false')
    expect(q('.full-view')).not.toBeNull() // toggling never leaves full view

    // Esc leaves full view: everything is back, the toggles go
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    flushSync()
    expect(q('.full-view')).toBeNull()
    expect(side().hidden).toBe(false)
    expect(members().hidden).toBe(false)
    expect(toggle('Channels')).toBeNull()

    // next time: hidden again by default (nothing remembered)
    openFull(SAM)
    await settle()
    expect(side().hidden).toBe(true)
    expect(members().hidden).toBe(true)
    expect(setItem.mock.calls.filter(([k]) => !/^pulse\.(stream|sounds|unsent|quickSwitch)/.test(String(k)))).toEqual([])
  })

  // #63: in full view the voice chat is the strip beside the stream, not the shell's right column
  it('full_view_shows_the_voice_chat_as_its_strip_and_the_right_column_holds_the_members', async () => {
    invoke.mockImplementation(async (cmd: string) => (cmd === 'list_messages' ? [] : undefined))
    localStorage.setItem('pulse.voiceChat', JSON.stringify({ open: true }))
    reloadVoiceChat()
    goTo('V1')
    await settle()
    expect(q('.right-panel aside.voice-chat')).not.toBeNull()
    expect(ui.openChannelId).toBe('V1')
    await streamApi.watch('V1', 'U2')
    openFull(SAM)
    await settle()
    await new Promise((r) => setTimeout(r, 150)) // the column's chat fades out
    flushSync()
    // one chat on screen: the strip, inside the full view
    expect(document.querySelectorAll('.chan.panel')).toHaveLength(1)
    expect(q('.full-view aside#stream-chat .chan.panel')).not.toBeNull()
    expect(q('.right-panel aside.voice-chat')).toBeNull()
    // its messages are on screen (alerts and jumps go by it)
    expect(ui.openChannelId).toBe('V1')
    // the corner toggle brings back the members list
    toggle('Members')!.click()
    flushSync()
    expect(q('.right-panel')!.hidden).toBe(false)
    expect(q('.right-panel aside.members')).not.toBeNull()
    // the strip closed: nothing of the chat on screen
    q('#stream-chat-toggle')!.click()
    await settle()
    expect(ui.openChannelId).toBeNull()
    // leaving full view with it open again: back in the column
    q('#stream-chat-toggle')!.click()
    await settle()
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', cancelable: true }))
    await settle()
    expect(q('.full-view')).toBeNull()
    expect(q('.right-panel aside.voice-chat')).not.toBeNull()
    expect(ui.openChannelId).toBe('V1')
  })

  it('a_click_on_the_strip_toggle_focuses_its_message_box', async () => {
    invoke.mockImplementation(async (cmd: string) => (cmd === 'list_messages' ? [] : undefined))
    goTo('V1')
    await streamApi.watch('V1', 'U2')
    openFull(SAM)
    await settle()
    const t = q('#stream-chat-toggle')!
    t.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, pointerType: 'mouse' }))
    t.click()
    await settle()
    expect(document.activeElement).toBe(q('.full-view aside#stream-chat [role="textbox"]'))
  })

  it('swapping_the_stream_in_full_view_keeps_the_panels_as_you_set_them', async () => {
    goTo('V1')
    await streamApi.watch('V1', 'U2')
    openFull(SAM)
    await settle()
    toggle('Channels')!.click()
    flushSync()
    expect(q('aside.side')!.hidden).toBe(false)
    // another stream in the same call takes full view from its toast's Watch (the watch cap ends sam's)
    watchStream('V1', 'U3')
    await settle()
    expect(callView.full).toEqual({ channelId: 'V1', streamer: 'U3' })
    expect(q('.full-view')).not.toBeNull()
    expect(q('aside.side')!.hidden).toBe(false)
  })

  it('switching_channel_in_full_view_brings_the_panels_back', async () => {
    goTo('V1')
    await streamApi.watch('V1', 'U2')
    openFull(SAM)
    await settle()
    goTo('T1')
    await settle()
    expect(q('aside.side')!.hidden).toBe(false)
    expect(q('.right-panel')!.hidden).toBe(false)
    expect(toggle('Channels')).toBeNull()
  })

  it('a_menu_left_open_in_a_panel_closes_when_full_view_hides_it_so_esc_still_leaves', async () => {
    goTo('V1')
    await streamApi.watch('V1', 'U2')
    await settle()
    q('.head-btn[data-menu-toggle]')!.click() // the server menu, open
    flushSync()
    expect(q('.menu[role="menu"]')).not.toBeNull()
    openFull(SAM)
    await settle()
    expect(q('.menu[role="menu"]')).toBeNull()
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', cancelable: true }))
    flushSync()
    expect(q('.full-view')).toBeNull()
  })

  it('focus_inside_a_panel_that_hides_moves_to_its_toggle', async () => {
    goTo('V1')
    await streamApi.watch('V1', 'U2')
    await settle()
    q('aside.side button')!.focus()
    expect(q('aside.side')!.contains(document.activeElement)).toBe(true)
    openFull(SAM)
    await settle()
    expect(document.activeElement).toBe(toggle('Channels'))
  })

  it('focus_inside_the_member_list_as_it_hides_moves_to_the_members_toggle', async () => {
    goTo('V1')
    await streamApi.watch('V1', 'U2')
    await settle()
    const inMembers = q('.right-panel')!.querySelector<HTMLElement>('button, [tabindex]')!
    inMembers.focus()
    expect(q('.right-panel')!.contains(document.activeElement)).toBe(true)
    openFull(SAM)
    await settle()
    expect(document.activeElement).toBe(toggle('Members'))
  })

  it('full_view_away_from_the_call_view_hides_nothing', async () => {
    // full view kept for the mini-preview while you read a text channel: the panels stay
    goTo('V1')
    await streamApi.watch('V1', 'U2')
    openFull(SAM)
    await settle()
    goTo('T1')
    await settle()
    expect(q('.mini-preview')).not.toBeNull()
    expect(q('aside.side')!.hidden).toBe(false)
    expect(q('.right-panel')!.hidden).toBe(false)
    expect(toggle('Channels')).toBeNull()
  })

  it('stream_end_closes_full_view_and_mini_preview', async () => {
    goTo('V1')
    await streamApi.watch('V1', 'U2')
    openFull(SAM)
    await settle()
    expect(q('.full-view')).not.toBeNull()
    emit('pulse://event', { t: 'StreamEnded', d: { channel_id: 'V1', user_id: 'U2', reason: 'stopped' } })
    await settle()
    expect(q('.full-view')).toBeNull()
    expect(q('.mini-preview')).toBeNull()
    expect(callView.full).toBeNull()
    expect(document.body.textContent).toContain('sam stopped streaming')

    // the same from the mini-preview
    emit('pulse://event', { t: 'StreamStarted', d: { stream: samStream } })
    await streamApi.watch('V1', 'U2')
    openFull(SAM)
    goTo('T1')
    await settle()
    expect(q('.mini-preview')).not.toBeNull()
    emit('pulse://event', { t: 'StreamEnded', d: { channel_id: 'V1', user_id: 'U2', reason: 'stopped' } })
    await settle()
    expect(q('.mini-preview')).toBeNull()
    expect(callView.full).toBeNull()
    expect(document.querySelectorAll('video')).toHaveLength(0)
    expect(document.body.textContent).toContain('sam stopped streaming')
  })

  it('leaving_voice_drops_the_mini_preview_and_its_element', async () => {
    goTo('V1')
    await streamApi.watch('V1', 'U2')
    await settle()
    goTo('T1')
    await settle()
    expect(q('.mini-preview video')).not.toBeNull()
    emit('voice://event', { kind: 'state', channel_id: null, connection: 'disconnected', controls: flags })
    await settle()
    expect(q('.mini-preview')).toBeNull()
    expect(document.querySelectorAll('video').length).toBe(0)
    expect(stream.watching).toEqual([])
  })

  it('toast_watch_opens_the_call_view_in_full_view', async () => {
    // somewhere else: Home
    expect(q('.full-view')).toBeNull()
    showToast({ channelId: 'V1', streamer: 'U2', name: 'sam' })
    flushSync()
    button('Watch')!.click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('stream_watch', { channelId: 'V1', streamer: 'U2' })
    expect(callView.full).toEqual(SAM)
    expect(q('.full-view .stage video')).not.toBeNull()
    expect(q('.mini-preview')).toBeNull()
    expect(room.setLayer).toHaveBeenLastCalledWith('U2', 'high')
  })

  it('join_and_watch_joins_then_opens_the_stream', async () => {
    voice.channelId = null
    voice.connection = 'disconnected'
    voice.stage = { stage: 'disconnected', attempt: null }
    showToast({ channelId: 'V1', streamer: 'U2', name: 'sam' })
    flushSync()
    button('Join & watch')!.click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('join_voice', expect.objectContaining({ channelId: 'V1' }))
    expect(invoke.mock.calls.filter(([c]) => c === 'stream_watch')).toHaveLength(0)
    // not yet connected: still waiting
    emit('voice://event', { kind: 'state', channel_id: 'V1', connection: 'connecting', controls: flags })
    await settle()
    expect(invoke.mock.calls.filter(([c]) => c === 'stream_watch')).toHaveLength(0)

    emit('voice://event', { kind: 'state', channel_id: 'V1', connection: 'connected', controls: flags })
    await settle()
    expect(invoke).toHaveBeenCalledWith('stream_watch', { channelId: 'V1', streamer: 'U2' })
    expect(callView.joinThenWatch).toBeNull()
    expect(callView.full).toEqual(SAM)
    expect(q('.full-view .stage video')).not.toBeNull()
  })

  it('a_failed_join_drops_the_pending_watch', async () => {
    voice.channelId = null
    voice.connection = 'disconnected'
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'join_voice') throw 'no route to the voice server'
    })
    showToast({ channelId: 'V1', streamer: 'U2', name: 'sam' })
    flushSync()
    button('Join & watch')!.click()
    await settle()
    expect(voice.joinFailed).not.toBeNull()
    expect(callView.joinThenWatch).toBeNull()
    // joining later by hand doesn't start the old watch
    emit('voice://event', { kind: 'state', channel_id: 'V1', connection: 'connected', controls: flags })
    await settle()
    expect(invoke.mock.calls.filter(([c]) => c === 'stream_watch')).toHaveLength(0)
  })

  it('joining_somewhere_else_meanwhile_drops_the_pending_watch', async () => {
    voice.channelId = null
    voice.connection = 'disconnected'
    showToast({ channelId: 'V1', streamer: 'U2', name: 'sam' })
    flushSync()
    button('Join & watch')!.click()
    await settle()
    voice.joining = 'V2'
    await settle()
    expect(callView.joinThenWatch).toBeNull()
  })

  it('the_live_popover_watches_in_the_call_and_joins_first_from_outside', async () => {
    const anchor = document.createElement('span')
    document.body.append(anchor)
    const pop = mount(LivePopover, { target: document.body, props: { channelId: 'V1', userId: 'U2', anchor, onClose: () => {} } })
    flushSync()
    expect(q('[aria-label="Live stream"] .watch')!.textContent).toBe('Watch')
    unmount(pop)

    voice.channelId = null
    voice.connection = 'disconnected'
    const pop2 = mount(LivePopover, { target: document.body, props: { channelId: 'V1', userId: 'U2', anchor, onClose: () => {} } })
    flushSync()
    const watch = q('[aria-label="Live stream"] .watch')!
    expect(watch.textContent).toBe('Join & watch')
    watch.click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('join_voice', expect.objectContaining({ channelId: 'V1' }))
    expect(callView.joinThenWatch).toEqual(SAM)
    unmount(pop2)
  })

  it('the_live_popover_watches_as_a_ghost_from_outside_when_the_stream_allows_it', async () => {
    app.state = applyEvent(app.state, { t: 'StreamUpdated', d: { stream: { ...samStream, ghosts: 2, settings: { ...samStream.settings, allow_ghosts: true } } } }, 0)
    voice.channelId = null
    voice.connection = 'disconnected'
    const anchor = document.createElement('span')
    document.body.append(anchor)
    const pop = mount(LivePopover, { target: document.body, props: { channelId: 'V1', userId: 'U2', anchor, onClose: () => {} } })
    flushSync()
    const box = q('[aria-label="Live stream"]')!
    expect(box.textContent).toContain('👻 2')
    expect(box.textContent).toContain("You'll watch as a ghost")
    const watch = box.querySelector<HTMLElement>('.watch')!
    expect(watch.textContent).toBe('Watch')
    watch.click()
    await settle()
    expect(invoke.mock.calls.filter(([c]) => c === 'join_voice')).toHaveLength(0)
    expect(invoke).toHaveBeenCalledWith('stream_watch', { channelId: 'V1', streamer: 'U2', ghost: true })
    expect(callView.full).toEqual(SAM)
    unmount(pop)
  })

  it('join_and_watch_twice_joins_once', async () => {
    voice.channelId = null
    voice.connection = 'disconnected'
    joinAndWatch('V1', 'U2')
    joinAndWatch('V1', 'U2')
    await settle()
    expect(invoke.mock.calls.filter(([c]) => c === 'join_voice')).toHaveLength(1)
    expect(callView.joinThenWatch).toEqual(SAM)
  })
})

describe('Go Live cleanup', () => {
  const user = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
  const mountShell = () => {
    cmp = mount(Shell, { target: document.body, props: { user, onLogout: () => {} } })
    flushSync()
  }
  afterEach(() => {
    closeGoLive()
    voice.channelId = null
    voice.connection = 'disconnected'
  })

  it('losing_the_connection_drops_the_picked_capture', async () => {
    voice.channelId = 'V1'
    voice.connection = 'connected'
    const stop = vi.fn()
    goLiveUi.picked = { track: { stop } } as never
    goLiveUi.channelId = 'V1'
    mountShell()
    expect(goLiveUi.picked).not.toBeNull()
    expect(stop).not.toHaveBeenCalled()
    voice.connection = 'reconnecting'
    await tick()
    flushSync()
    expect(stop).toHaveBeenCalledTimes(1)
    expect(goLiveUi.picked).toBeNull()
  })

  it('a_picker_error_does_not_outlive_the_call', async () => {
    voice.channelId = 'V1'
    voice.connection = 'connected'
    goLiveUi.error = 'no screen'
    goLiveUi.channelId = 'V1'
    mountShell()
    expect(goLiveUi.error).toBe('no screen')
    voice.channelId = null
    voice.connection = 'disconnected'
    await tick()
    flushSync()
    expect(goLiveUi.error).toBe('')
    expect(goLiveUi.channelId).toBeNull()
  })
})

describe('jump to a message', () => {
  it('a jump request switches to its channel', async () => {
    const { ui } = await import('../lib/store.svelte')
    const { testMe: me, testReady } = await import('../lib/testReady')
    app.state = applyReady(emptyState(), testReady([{ id: 'Q1', server_id: 'S1', kind: 'quotes', name: 'quotes', position: 0 }]))
    cmp = mount(Shell, { target: document.body, props: { user: me, onLogout: () => {} } })
    flushSync()
    expect(document.querySelector('.chan .head .title')).toBeNull()
    ui.goTo = 'Q1'
    flushSync()
    expect(document.querySelector('.chan .head .title')?.textContent).toBe('quotes')
    expect(ui.goTo).toBeNull()
  })
})

describe('a deleted channel', () => {
  it('moves you from it to another channel of the server and says why', async () => {
    const { ui } = await import('../lib/store.svelte')
    const { testMe: me, testReady } = await import('../lib/testReady')
    app.state = applyReady(emptyState(), testReady([
      { id: 'T1', server_id: 'S1', kind: 'text', name: 'general', position: 0 },
      { id: 'T2', server_id: 'S1', kind: 'text', name: 'old', position: 1 },
    ]))
    cmp = mount(Shell, { target: document.body, props: { user: me, onLogout: () => {} } })
    flushSync()
    ui.goTo = 'T2'
    flushSync()
    expect(document.querySelector('.chan .head .title')?.textContent).toBe('old')
    app.state = applyEvent(app.state, { t: 'ChannelDeleted', d: { channel_id: 'T2', server_id: 'S1' } }, Date.now())
    flushSync()
    expect(document.querySelector('.chan .head .title')?.textContent).toBe('general')
    expect(document.querySelector('.notice')?.textContent).toBe('#old was deleted.')
  })
})

describe('a deleted server (#184)', () => {
  it('takes you Home from it and says why, after its channels go one by one', async () => {
    const { ui } = await import('../lib/store.svelte')
    const { testMe: me, testReady } = await import('../lib/testReady')
    app.state = applyReady(emptyState(), testReady([
      { id: 'T1', server_id: 'S1', kind: 'text', name: 'general', position: 0 },
      { id: 'T2', server_id: 'S1', kind: 'text', name: 'old', position: 1 },
    ]))
    cmp = mount(Shell, { target: document.body, props: { user: me, onLogout: () => {} } })
    flushSync()
    ui.goTo = 'T2'
    flushSync()
    expect(document.querySelector('.chan .head .title')?.textContent).toBe('old')
    // what the server sends: each channel's ChannelDeleted, then ServerDeleted
    for (const id of ['T1', 'T2']) {
      app.state = applyEvent(app.state, { t: 'ChannelDeleted', d: { channel_id: id, server_id: 'S1' } }, Date.now())
      flushSync()
    }
    app.state = applyEvent(app.state, { t: 'ServerDeleted', d: { server_id: 'S1' } }, Date.now())
    flushSync()
    expect(document.querySelector('.chan .head .title')).toBeNull()
    expect([...document.querySelectorAll('nav[aria-label="Servers"] .tab')].map((t) => t.textContent?.trim())).not.toContain('Main')
    expect(document.querySelector('.panel-head')?.textContent).toContain('Direct messages')
    expect(document.querySelector('.notice')?.textContent).toBe('Main was deleted.')
  })
})

describe('Quote of the Year', () => {
  it('a live vote shows its banner over every channel of its server', async () => {
    const { ui } = await import('../lib/store.svelte')
    const { testMe: me, testQoty: liveVote, testReady } = await import('../lib/testReady')
    app.state = applyReady(emptyState(), testReady([
      { id: 'T1', server_id: 'S1', kind: 'text', name: 'general', position: 0 },
      { id: 'Q1', server_id: 'S1', kind: 'quotes', name: 'quotes', position: 1 },
    ], { qoty: [liveVote()] }))
    cmp = mount(Shell, { target: document.body, props: { user: me, onLogout: () => {} } })
    flushSync()
    ui.goTo = 'T1'
    flushSync()
    expect(document.querySelector('main .qoty-banner')?.textContent).toContain('Quote of the Year 2026')
    ui.goTo = 'Q1'
    flushSync()
    expect(document.querySelector('main .qoty-banner')).toBeTruthy()
    app.state = { ...app.state, qoty: { S1: liveVote({ ended_at: '2026-10-27T11:00:00Z', end_reason: 'early' }) } }
    flushSync()
    expect(document.querySelector('.qoty-banner')).toBeNull()
  })

  it('Start Quote of the Year opens the start dialog for the server', async () => {
    const { ui } = await import('../lib/store.svelte')
    const { testMe: me, testReady } = await import('../lib/testReady')
    app.state = applyReady(emptyState(), testReady([{ id: 'Q1', server_id: 'S1', kind: 'quotes', name: 'quotes', position: 0 }]))
    cmp = mount(Shell, { target: document.body, props: { user: me, onLogout: () => {} } })
    flushSync()
    ui.goTo = 'Q1'
    flushSync()
    ;[...document.querySelectorAll('button')].find((b) => b.textContent === 'Start Quote of the Year')!.click()
    flushSync()
    expect(document.querySelector('[role=dialog][aria-label="Start Quote of the Year"]')).toBeTruthy()
    document.querySelector<HTMLButtonElement>('[role=dialog] button[aria-label="Cancel"]')!.click()
    flushSync()
    expect(document.querySelector('[role=dialog][aria-label="Start Quote of the Year"]')).toBeNull()
  })
})

describe('clips being sent', () => {
  it('keeps the clips being sent: the ones left over at start, then each clip://job', async () => {
    const job = (id: string, over: Record<string, unknown> = {}) => ({
      id, channel_id: 'C1', stage: 'failed', progress: 0, error: null, caption: '', game_id: null, kept_ms: 9000, thumb_path: null, message_id: null, ...over,
    })
    let answer: (jobs: unknown[]) => void = () => {}
    invoke.mockImplementation(async (cmd: string) => (cmd === 'clip_jobs' ? new Promise((r) => (answer = r)) : undefined))
    app.state = emptyState()
    cmp = mount(Shell, { target: document.body, props: { user: { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }, onLogout: () => {} } })
    flushSync()
    await new Promise((r) => setTimeout(r, 0))
    expect(invoke.mock.calls.filter((c) => c[0] === 'clip_jobs')).toHaveLength(1)
    // an event that beats the startup list is newer than it
    handlers.get('clip://job')!({ payload: job('J2', { stage: 'uploading', progress: 0.5 }) })
    answer([job('J1'), job('J2')])
    await new Promise((r) => setTimeout(r, 0))
    expect(app.state.clipJobs.J1?.stage).toBe('failed')
    expect(app.state.clipJobs.J2?.stage).toBe('uploading')
    handlers.get('clip://job')!({ payload: job('J1', { stage: 'done', progress: 1 }) })
    expect(app.state.clipJobs.J1).toBeUndefined()
  })

  it('a posted clip keeps its card a moment, so the clip can take its place', async () => {
    const job = (over: Record<string, unknown> = {}) => ({
      id: 'J1', channel_id: 'C1', stage: 'posting', progress: 1, error: null, caption: '', game_id: null, kept_ms: 9000, thumb_path: null, message_id: null, ...over,
    })
    invoke.mockImplementation(async (cmd: string) => (cmd === 'clip_jobs' ? [] : undefined))
    app.state = emptyState()
    vi.useFakeTimers()
    try {
      cmp = mount(Shell, { target: document.body, props: { user: { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }, onLogout: () => {} } })
      flushSync()
      await vi.advanceTimersByTimeAsync(0)
      handlers.get('clip://job')!({ payload: job() })
      handlers.get('clip://job')!({ payload: job({ stage: 'done', message_id: 'M9' }) })
      expect(app.state.clipJobs.J1?.stage).toBe('done')
      await vi.advanceTimersByTimeAsync(4_900)
      expect(app.state.clipJobs.J1).toBeDefined()
      await vi.advanceTimersByTimeAsync(200)
      expect(app.state.clipJobs.J1).toBeUndefined()
    } finally {
      vi.useRealTimers()
    }
  })

  it("still loads the clips being sent when it can't listen for them", async () => {
    listenFails.add('clip://job')
    try {
      invoke.mockImplementation(async (cmd: string) =>
        cmd === 'clip_jobs' ? [{ id: 'J1', channel_id: 'C1', stage: 'failed', progress: 0, error: null, caption: '', game_id: null, kept_ms: 9000, thumb_path: null, message_id: null }] : undefined)
      app.state = emptyState()
      cmp = mount(Shell, { target: document.body, props: { user: { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }, onLogout: () => {} } })
      flushSync()
      await new Promise((r) => setTimeout(r, 0))
      await new Promise((r) => setTimeout(r, 0))
      expect(app.state.clipJobs.J1?.stage).toBe('failed')
    } finally {
      listenFails.clear()
    }
  })
})

describe('trimming history on leave', () => {
  const msgs = (channel_id: string, n: number) => Array.from({ length: n }, (_, i) => ({
    id: `M${String(i).padStart(4, '0')}`, channel_id, author_id: 'U2', kind: 'normal' as const, content: `m${i}`, reply_to_id: null,
    created_at: '2026-10-01T00:00:00.000Z', edited_at: null, deleted: false, mentions: [], reactions: [], thread_root_id: null, quote: null, qoty_card: null, clip: null, embeds: [],
  }))

  it('leaving a channel trims it to its newest messages', async () => {
    const { ui } = await import('../lib/store.svelte')
    const { addHistory, KEEP_ON_LEAVE } = await import('../lib/state')
    const { testMe: me, testReady } = await import('../lib/testReady')
    app.state = applyReady(emptyState(), testReady([
      { id: 'T1', server_id: 'S1', kind: 'text', name: 'general', position: 0 },
      { id: 'T2', server_id: 'S1', kind: 'text', name: 'random', position: 1 },
    ]))
    cmp = mount(Shell, { target: document.body, props: { user: me, onLogout: () => {} } })
    ui.goTo = 'T1'
    flushSync()
    await tick()
    app.state = addHistory(app.state, 'T1', msgs('T1', KEEP_ON_LEAVE + 50))
    ui.goTo = 'T2'
    flushSync()
    expect(app.state.messages['T1']).toHaveLength(KEEP_ON_LEAVE)
    expect(app.state.messages['T1'][0].id).toBe('M0050') // the newest are kept
  })

  it('signing out does not write the old session back over the reset state', async () => {
    const { resetState, ui } = await import('../lib/store.svelte')
    const { addHistory } = await import('../lib/state')
    const { testMe: me, testReady } = await import('../lib/testReady')
    app.state = applyReady(emptyState(), testReady([{ id: 'T1', server_id: 'S1', kind: 'text', name: 'general', position: 0 }]))
    cmp = mount(Shell, { target: document.body, props: { user: me, onLogout: () => {} } })
    ui.goTo = 'T1'
    flushSync()
    await tick()
    app.state = addHistory(app.state, 'T1', msgs('T1', 150))
    flushSync()
    // App's onLogout resets the state and drops the shell in one go: the shell's teardown runs
    // before that update settles, where Svelte hands it the state from before the reset
    resetState()
    unmount(cmp)
    cmp = undefined
    expect(app.state.me).toBeNull()
    expect(app.state.messages).toEqual({})
  })
})

// Signing out resets the app's data in the same update that removes the shell (App: user = null; resetState()).
// For that moment the open channel's view is still mounted while its channel is gone from the data: nothing may
// read the channel then, or the error leaves the page blank for the next sign-in (owner, 2026-11-03).
describe('signing out with a channel open', () => {
  const me = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
  const ready: Ready = {
    me,
    servers: [{ id: 'S1', name: 'Main', icon_hash: null }],
    channels: [
      { id: 'T1', server_id: 'S1', kind: 'text', name: 'general', position: 0 },
      { id: 'V1', server_id: 'S1', kind: 'voice', name: 'Lounge', position: 1 },
      { id: 'K1', server_id: 'S1', kind: 'clips', name: 'clips', position: 2 },
    ],
    members: [{ server_id: 'S1', members: [{ user: me, nickname: null }] }],
    dm_members: [], voice: [], people: [], read_states: [], mutes: [], hidden: [], latest: [], streams: [], custom_emoji: [], frequent_emoji: [], qoty: [], speaker_links: [], thread_seen: [], games: [], clip_views: [], private_nicknames: [], sounds: [], sound_favourites: [], frequent_sounds: [], server_voice: [], status: { status: 'online', dnd_until: null }, appear_offline_to: [], live_notify: [],
  }
  const open = (name: string) => {
    ;[...document.querySelectorAll<HTMLElement>('.row')].find((r) => r.textContent?.includes(name))!.dispatchEvent(new MouseEvent(name === 'Lounge' ? 'dblclick' : 'click', { bubbles: true }))
    flushSync()
  }

  it.each(['general', 'Lounge', 'clips'])('the %s view lets go without reading the vanished channel', async (name) => {
    app.state = applyReady(emptyState(), ready)
    cmp = mount(Shell, { target: document.body, props: { user: me, onLogout: () => {} } })
    flushSync()
    ;[...document.querySelectorAll<HTMLButtonElement>('.tab')].find((t) => t.textContent!.includes('Main'))!.click()
    flushSync()
    if (name !== 'general') open(name)
    await tick()
    const errors: unknown[] = []
    const onError = (e: ErrorEvent) => errors.push(e.error)
    window.addEventListener('error', onError)
    try {
      // the sign-out's reset, before the shell goes
      expect(() => {
        app.state = emptyState()
        flushSync()
      }).not.toThrow()
      await tick()
      expect(errors).toEqual([])
    } finally {
      window.removeEventListener('error', onError)
    }
  })
})

describe('resizable columns (#160)', () => {
  const alex = { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }
  const handle = (label: string) => document.querySelector<HTMLElement>(`[role=separator][aria-label="${label}"]`)
  const key = (el: Element, k: string) => {
    el.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true }))
    flushSync()
  }
  const showServer = () => {
    app.state = { ...emptyState(), me: alex, servers: [{ id: 'S1', name: 'Alpha', icon_hash: null }] }
    cmp = mount(Shell, { target: document.body, props: { user: alex, onLogout: () => {} } })
    flushSync()
    ;[...document.querySelectorAll<HTMLButtonElement>('.tab')].find((t) => t.textContent!.includes('Alpha'))!.click()
    flushSync()
  }
  beforeEach(() => {
    localStorage.removeItem('pulse.columns')
    reloadColumns()
  })

  it('a handle sits on each side of the main area, controlling its column', () => {
    showServer()
    const left = handle('Resize the channel list')!
    const right = handle('Resize the members list')!
    expect(left.getAttribute('aria-controls')).toBe(document.querySelector('aside.side')!.id)
    expect(right.getAttribute('aria-controls')).toBe(document.querySelector('.right-panel')!.id)
    expect(left.getAttribute('aria-valuenow')).toBe('264')
    expect(right.getAttribute('aria-valuenow')).toBe('240')
    // the handles sit between the panels, in the body's row
    const row = [...document.querySelector('.body')!.children]
    expect(row.indexOf(left)).toBe(row.indexOf(document.querySelector('aside.side')!) + 1)
    expect(row.indexOf(right)).toBe(row.indexOf(document.querySelector('.right-panel')!) - 1)
  })

  it('resizing the channel list widens it and is remembered; a double-click puts the default back', () => {
    showServer()
    const side = document.querySelector<HTMLElement>('aside.side')!
    key(handle('Resize the channel list')!, 'ArrowRight')
    expect(side.style.width).toBe('280px')
    expect(JSON.parse(localStorage.getItem('pulse.columns')!).left).toBe(280)
    handle('Resize the channel list')!.dispatchEvent(new MouseEvent('dblclick', { bubbles: true }))
    flushSync()
    expect(side.style.width).toBe('264px')
    expect(JSON.parse(localStorage.getItem('pulse.columns')!).left).toBeNull()
  })

  it('a resized members list reaches the list; unsaved, the list keeps its own default', () => {
    showServer()
    const panel = document.querySelector<HTMLElement>('.right-panel')!
    expect(panel.style.getPropertyValue('--right-w')).toBe('')
    key(handle('Resize the members list')!, 'ArrowLeft')
    expect(panel.style.getPropertyValue('--right-w')).toBe('256px')
  })

  it('Home has no right column, so no right handle', () => {
    app.state = { ...emptyState(), me: alex }
    cmp = mount(Shell, { target: document.body, props: { user: alex, onLogout: () => {} } })
    flushSync()
    expect(handle('Resize the channel list')).not.toBeNull()
    expect(handle('Resize the members list')).toBeNull()
  })
})

describe('voice channel chat (#226)', () => {
  const handle = (label: string) => document.querySelector<HTMLElement>(`[role=separator][aria-label="${label}"]`)
  const q = (sel: string) => document.querySelector<HTMLElement>(sel)
  const show = () => {
    app.state = applyReady(emptyState(), testReady([
      { id: 'T1', server_id: 'S1', kind: 'text', name: 'general', position: 0 },
      { id: 'V1', server_id: 'S1', kind: 'voice', name: 'Lounge', position: 1 },
    ]))
    cmp = mount(Shell, { target: document.body, props: { user: testMe, onLogout: () => {} } })
    flushSync()
    ;[...document.querySelectorAll<HTMLButtonElement>('.tab')].find((t) => t.textContent!.includes('Main'))!.click()
    flushSync()
  }
  beforeEach(() => {
    localStorage.removeItem('pulse.columns')
    localStorage.removeItem('pulse.voiceChat')
    reloadColumns()
    reloadVoiceChat()
  })

  it('a search hit in a voice channel opens its chat in the results’ place, and the message is asked for there', async () => {
    const { runSearch } = await import('../lib/search.svelte')
    const { jumpTo } = await import('../lib/jump')
    const hit = { id: 'M9', channel_id: 'V1', author_id: 'U1', kind: 'normal', content: 'pineapple', reply_to_id: null, created_at: '2026-11-06T10:00:00Z', edited_at: null, deleted: false, reactions: [], mentions: [], embeds: [] }
    invoke.mockImplementation(async (cmd: string) =>
      cmd === 'search_messages' ? { hits: [], total: 0, next_cursor: null } : cmd === 'list_messages_around' ? [hit] : cmd === 'list_messages' ? [] : undefined)
    const scrolled = vi.fn()
    Element.prototype.scrollIntoView = scrolled
    show()
    await runSearch({ kind: 'server', serverId: 'S1' }, 'pineapple', [])
    flushSync()
    expect(q('.right-panel aside.voice-chat')).toBeNull()
    await jumpTo('V1', 'M9')
    flushSync()
    await tick()
    expect(q('.right-panel aside.voice-chat')).not.toBeNull()
    expect(ui.openChannelId).toBe('V1')
    expect(app.state.messages['V1']?.some((m) => m.id === 'M9')).toBe(true)
    // the chat scrolls to it and flashes it
    for (let i = 0; i < 4; i++) await tick()
    expect(q('aside.voice-chat [data-message-id="M9"]')?.classList.contains('jump-flash')).toBe(true)
    expect(scrolled).toHaveBeenCalled()
  })

  // #230: the results wait behind the chat instead of closing
  it('the results it replaced come back when the chat closes; a new search takes the column back', async () => {
    const { runSearch, search } = await import('../lib/search.svelte')
    const { jumpTo } = await import('../lib/jump')
    const hit = { id: 'M9', channel_id: 'V1', author_id: 'U1', kind: 'normal', content: 'pineapple', reply_to_id: null, created_at: '2026-11-06T10:00:00Z', edited_at: null, deleted: false, reactions: [], mentions: [], embeds: [] }
    invoke.mockImplementation(async (cmd: string) =>
      cmd === 'search_messages' ? { hits: [{ message: hit, text: 'pineapple', highlights: [], root: null }], total: 1, next_cursor: null }
        : cmd === 'list_messages_around' ? [hit] : cmd === 'list_messages' ? [] : undefined)
    Element.prototype.scrollIntoView = vi.fn()
    show()
    await runSearch({ kind: 'server', serverId: 'S1' }, 'pineapple', [])
    flushSync()
    await jumpTo('V1', 'M9')
    flushSync()
    await tick()
    expect(q('.right-panel aside.voice-chat')).not.toBeNull()
    expect(q('.right-panel .search-results')).toBeNull()
    expect(search.open, 'kept, not closed').toBe(true)
    expect(handle('Resize the chat')).not.toBeNull()
    q('[aria-label="Close chat"]')!.click()
    flushSync()
    await tick()
    expect(q('aside.voice-chat')).toBeNull()
    expect(q('.right-panel .search-results')?.textContent).toContain('1 result')
    // again, then a new search: the results take the column back, the chat stays open behind them
    await jumpTo('V1', 'M9')
    flushSync()
    await tick()
    expect(q('.right-panel aside.voice-chat')).not.toBeNull()
    await runSearch({ kind: 'server', serverId: 'S1' }, 'mango', [])
    flushSync()
    expect(q('.right-panel .search-results')).not.toBeNull()
    expect(q('aside.voice-chat')).toBeNull()
    expect(voiceChat.open).toBe(true)
  })

  it("the row's chat icon opens the stage with the chat beside it, in the members list's place, without joining", async () => {
    show()
    q('.chat-btn')!.click()
    flushSync()
    await tick()
    expect(q('.content .head .title')?.textContent).toBe('Lounge')
    const panel = q('.right-panel aside.voice-chat')!
    expect(panel.getAttribute('aria-label')).toBe('Chat')
    expect(panel.classList.contains('fade-in')).toBe(true)
    expect(q('.right-panel aside.members')).toBeNull()
    expect(ui.openChannelId).toBe('V1')
    expect(invoke.mock.calls.some((c) => c[0] === 'join_voice')).toBe(false)
    expect(voiceChat.open).toBe(true)
  })

  it('the stage toggle and the close button both close it; the members list comes back and the focus goes to the toggle', async () => {
    show()
    q('.chat-btn')!.click()
    flushSync()
    await tick()
    q('[aria-label="Close chat"]')!.focus()
    q('[aria-label="Close chat"]')!.click()
    flushSync()
    await tick()
    expect(q('aside.voice-chat')).toBeNull()
    expect(q('.right-panel aside.members')).not.toBeNull()
    expect(document.activeElement).toBe(q('#voice-chat-toggle'))
    expect(ui.openChannelId).toBeNull()
    q('#voice-chat-toggle')!.click()
    flushSync()
    expect(q('aside.voice-chat')).not.toBeNull()
    q('#voice-chat-toggle')!.click()
    flushSync()
    expect(q('aside.voice-chat')).toBeNull()
  })

  it('it keeps its own width, apart from the members list, and stays open for the next stage', async () => {
    show()
    q('.chat-btn')!.click()
    flushSync()
    const h = handle('Resize the chat')!
    expect(h.getAttribute('aria-valuenow')).toBe('360')
    h.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowLeft', bubbles: true, cancelable: true }))
    flushSync()
    expect(JSON.parse(localStorage.getItem('pulse.columns')!)).toEqual({ left: null, right: null, chat: 376, strip: null })
    expect(q('.right-panel')!.style.getPropertyValue('--right-w')).toBe('376px')
    // a text channel shows the members list at its own width
    q('.row[data-channel="T1"]')!.click()
    flushSync()
    expect(q('.right-panel')!.style.getPropertyValue('--right-w')).toBe('')
    expect(handle('Resize the members list')).not.toBeNull()
    // back on the stage (a double-click on the row opens it), the chat is still open
    q('.row[data-channel="V1"]')!.dispatchEvent(new MouseEvent('dblclick', { bubbles: true }))
    flushSync()
    expect(q('aside.voice-chat')).not.toBeNull()
  })
})

// #220: which call you're in shows from anywhere: its server's tab (or Home, for a DM or group call) wears the
// accent ring and a speaker badge
describe('the call you are in', () => {
  const mountIn = async (channelId: string | null) => {
    const { testReady } = await import('../lib/testReady')
    app.state = applyReady(emptyState(), testReady([
      { id: 'V1', server_id: 'S1', kind: 'voice', name: 'Lounge', position: 0 },
      { id: 'D1', server_id: null, kind: 'dm', name: null, position: 0 },
    ], { servers: [{ id: 'S1', name: 'Main', icon_hash: null }, { id: 'S2', name: 'Other', icon_hash: null }] }))
    voice.channelId = channelId
    voice.connection = channelId ? 'connected' : 'disconnected'
    cmp = mount(Shell, { target: document.body, props: { user: { id: 'U1', username: 'alex', display_name: 'alex', avatar_hash: null, avatar_still: null, style: { font: null, colors: [] } }, onLogout: () => {} } })
    flushSync()
  }
  const tab = (name: string) => [...document.querySelectorAll<HTMLButtonElement>('.tab')].find((t) => t.textContent!.includes(name))!
  afterEach(() => {
    voice.channelId = null
    voice.connection = 'disconnected'
  })

  it('a server call badges that server\'s tab, and only it', async () => {
    await mountIn('V1')
    expect(tab('Main').querySelector('.call-here .call-badge svg')).not.toBeNull()
    expect(tab('Main').getAttribute('aria-label')).toBe('Main (you\'re in a call here)')
    expect(tab('Other').querySelector('.call-here')).toBeNull()
    expect(tab('Home').querySelector('.call-here')).toBeNull()
  })

  it('a DM or group call badges Home', async () => {
    await mountIn('D1')
    expect(tab('Home').querySelector('.call-here .call-badge svg')).not.toBeNull()
    expect(tab('Main').querySelector('.call-here')).toBeNull()
  })

  it('no call, no badge', async () => {
    await mountIn(null)
    expect(document.querySelector('.call-here')).toBeNull()
  })
})
