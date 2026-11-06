/// <reference types="node" />
// The app's version lives in one place, the workspace Cargo.toml (#82); the patch notes name the same one.
import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import { patchNotes, previousPatchNotes } from './patchNotes'

const read = (rel: string) => readFileSync(new URL(rel, import.meta.url), 'utf8')

describe('patch notes version (#82)', () => {
  it("patch notes name the app's version", () => {
    const v = /\[workspace\.package\][^[]*?\nversion = "([^"]+)"/.exec(read('../../../Cargo.toml'))?.[1]
    expect(v).toMatch(/^\d+\.\d+\.\d+$/)
    expect(patchNotes.version).toBe(v)
  })

  it('tauri.conf.json carries no version of its own (Tauri reads Cargo.toml)', () => {
    expect(JSON.parse(read('../../src-tauri/tauri.conf.json')).version).toBeUndefined()
  })
})

// #235: the release before stays in the dialog, collapsed. Each release moves the current notes there.
describe('previous patch notes (#235)', () => {
  const parts = (v: string) => v.split('.').map(Number)
  const isOlder = (a: string, b: string) => {
    const [x, y] = [parts(a), parts(b)]
    for (let i = 0; i < 3; i++) if (x[i] !== y[i]) return x[i] < y[i]
    return false
  }

  it('are an older release than the current notes, with their own id', () => {
    expect(previousPatchNotes).not.toBeNull()
    expect(previousPatchNotes!.id).not.toBe(patchNotes.id)
    expect(previousPatchNotes!.version).toMatch(/^\d+\.\d+\.\d+$/)
    expect(isOlder(previousPatchNotes!.version, patchNotes.version)).toBe(true)
  })
})
