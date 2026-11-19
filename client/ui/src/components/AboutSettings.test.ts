// @vitest-environment jsdom
// Settings › About (#506): the version and Check for updates (the updater's, UpdateSettings), the server you're
// connected to, and this version's patch notes.
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...a) }))

import AboutSettings from './AboutSettings.svelte'
import { patchNotes } from '../patchNotes'
import { patchNotesSeen } from '../lib/patchNotesSeen.svelte'
import { forgetInvitePreviews } from '../lib/invitePreviews.svelte'

let cmp: ReturnType<typeof mount> | undefined
const settle = async () => {
  for (let i = 0; i < 4; i++) await new Promise((r) => setTimeout(r, 0))
  flushSync()
}
beforeEach(() => {
  invoke.mockReset()
  invoke.mockImplementation(async (cmd: string) =>
    cmd === 'server_origin' ? 'https://chat.example.test' : cmd === 'update_info' ? { version: patchNotes.version, enabled: true, packageManager: false } : cmd === 'update_check' ? null : undefined)
})
afterEach(() => {
  if (cmp) unmount(cmp)
  cmp = undefined
  document.body.innerHTML = ''
  forgetInvitePreviews()
})

describe('Settings › About', () => {
  it('shows the version, the server, and checks for updates', async () => {
    cmp = mount(AboutSettings, { target: document.body })
    await settle()
    expect(document.body.textContent).toContain(`Pulse ${patchNotes.version}`)
    expect(document.querySelector('[data-server]')!.textContent).toBe('chat.example.test')
    ;[...document.querySelectorAll('button')].find((b) => b.textContent?.trim() === 'Check for updates')!.click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('update_check')
    expect(document.body.textContent).toContain("You're up to date.")
  })

  it("opens this version's patch notes, and counts them as seen", async () => {
    cmp = mount(AboutSettings, { target: document.body })
    await settle()
    ;[...document.querySelectorAll('button')].find((b) => b.textContent?.includes("What's new in"))!.click()
    flushSync()
    expect(document.querySelector('[role=dialog]')).not.toBeNull()
    expect(patchNotesSeen.id).toBe(patchNotes.id)
  })
})
