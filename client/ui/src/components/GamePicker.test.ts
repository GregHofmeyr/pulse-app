// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Game } from '../lib/protocol/Game'

const invoke = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({ invoke: (cmd: string, args?: never) => invoke(cmd, args) }))

import GamePicker from './GamePicker.svelte'
import { app } from '../lib/store.svelte'
import { emptyState } from '../lib/state'

const game = (id: string, name: string): Game => ({ id, server_id: 'S1', name, created_by: null, created_at: '2026-10-01T10:00:00.000Z' })

let cmps: ReturnType<typeof mount>[] = []
function show(props: { value?: string | null; guess?: string | null; onChange?: (id: string | null) => void } = {}) {
  const target = document.createElement('div')
  document.body.append(target)
  const onChange = props.onChange ?? vi.fn()
  cmps.push(mount(GamePicker, { target, props: { serverId: 'S1', value: props.value ?? null, guess: props.guess, onChange } }))
  flushSync()
  return { target, onChange }
}
const settle = () => new Promise((r) => setTimeout(r, 0))
const type = (root: Element, text: string) => {
  const i = root.querySelector('input')!
  i.focus()
  i.value = text
  i.dispatchEvent(new Event('input', { bubbles: true }))
  flushSync()
}
const options = (root: Element) => [...root.querySelectorAll<HTMLElement>('[role=option]')].map((o) => o.textContent?.trim())

beforeEach(() => {
  invoke.mockReset()
  app.state = { ...emptyState(), games: { S1: [game('G1', 'Overwatch 2'), game('G2', 'Rust')] } }
})
afterEach(() => { cmps.forEach((c) => unmount(c)); cmps = []; document.body.innerHTML = '' })

describe('GamePicker', () => {
  it('picks a known game ignoring case', () => {
    const { target, onChange } = show()
    type(target, 'overWATCH 2')
    expect(options(target)).toEqual(['Overwatch 2'])
    target.querySelector<HTMLElement>('[role=option]')!.click()
    expect(onChange).toHaveBeenCalledWith('G1')
    // a guess that differs only in case selects the game at once
    const second = show({ guess: 'rust' })
    expect(second.onChange).toHaveBeenCalledWith('G2')
  })

  it('adds a new game', async () => {
    invoke.mockImplementation(async (cmd) => (cmd === 'add_game' ? game('G3', 'Valorant') : undefined))
    const { target, onChange } = show()
    type(target, 'Valorant')
    expect(options(target)).toEqual(["Add 'Valorant'"])
    target.querySelector<HTMLElement>('[role=option]')!.click()
    await settle()
    expect(invoke).toHaveBeenCalledWith('add_game', { serverId: 'S1', name: 'Valorant' })
    expect(onChange).toHaveBeenCalledWith('G3')
  })

  it('pre-fills a new guess as an offer without adding it', async () => {
    const { target, onChange } = show({ guess: 'Valorant' })
    expect(options(target)).toEqual(["Add 'Valorant'"])
    expect(target.querySelector('input')!.value).toBe('Valorant')
    expect(onChange).not.toHaveBeenCalled()
    expect(invoke).not.toHaveBeenCalled()
  })

  it('shows the selected game and clears on empty input', () => {
    const { target, onChange } = show({ value: 'G2' })
    expect(target.querySelector('input')!.value).toBe('Rust')
    type(target, '')
    expect(onChange).toHaveBeenCalledWith(null)
  })
})
