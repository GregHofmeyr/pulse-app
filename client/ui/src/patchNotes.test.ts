/// <reference types="node" />
// The app's version lives in one place, the workspace Cargo.toml (#82); the patch notes name the same one.
import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import { patchNotes } from './patchNotes'

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
