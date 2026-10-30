/// <reference types="node" />
// UI sounds are played by the core (cpal) from the copies built into the Rust binary
// (src-tauri/sounds). An audio file anywhere Vite bundles from would ship them a second time.
import { existsSync, readdirSync } from 'node:fs'
import { describe, expect, it } from 'vitest'

const AUDIO = /\.(wav|ogg|mp3|flac|opus|m4a|aac)$/i
const audioUnder = (dir: URL): string[] =>
  existsSync(dir) ? (readdirSync(dir, { recursive: true }) as string[]).filter((f) => AUDIO.test(f)) : []

describe('sounds ship once', () => {
  it('no audio file is in public/ or src/, where the webview assets come from', () => {
    expect(audioUnder(new URL('../public/', import.meta.url))).toEqual([])
    expect(audioUnder(new URL('./', import.meta.url))).toEqual([])
  })

  it('the core still has them (the scan would pass vacuously if they had moved)', () => {
    expect(audioUnder(new URL('../../src-tauri/sounds/', import.meta.url)).length).toBeGreaterThan(0)
  })
})
