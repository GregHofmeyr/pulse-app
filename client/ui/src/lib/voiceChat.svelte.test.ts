// @vitest-environment jsdom
import { beforeEach, describe, expect, it } from 'vitest'
import { reloadVoiceChat, setVoiceChat, takeChatFocus, toggleVoiceChat, voiceChat } from './voiceChat.svelte'

beforeEach(() => {
  localStorage.clear()
  reloadVoiceChat()
})

describe('the voice chat panel (#226)', () => {
  it('starts closed', () => {
    expect(voiceChat.open).toBe(false)
  })

  it('open or closed is remembered on this device', () => {
    toggleVoiceChat()
    expect(voiceChat.open).toBe(true)
    expect(JSON.parse(localStorage.getItem('pulse.voiceChat')!)).toEqual({ open: true })
    voiceChat.open = false
    reloadVoiceChat()
    expect(voiceChat.open).toBe(true)
    toggleVoiceChat()
    reloadVoiceChat()
    expect(voiceChat.open).toBe(false)
  })

  it('garbled storage is closed', () => {
    localStorage.setItem('pulse.voiceChat', 'nope')
    reloadVoiceChat()
    expect(voiceChat.open).toBe(false)
    localStorage.setItem('pulse.voiceChat', '{"open":"yes"}')
    reloadVoiceChat()
    expect(voiceChat.open).toBe(false)
  })

  it('a click asks for the focus once, also when the panel is already open', () => {
    expect(takeChatFocus()).toBe(false)
    setVoiceChat(true, true)
    expect(takeChatFocus()).toBe(true)
    expect(takeChatFocus()).toBe(false)
    setVoiceChat(true, true)
    expect(takeChatFocus()).toBe(true)
    // closing never asks; opening without a click doesn't either
    toggleVoiceChat(true)
    setVoiceChat(true)
    expect(takeChatFocus()).toBe(false)
  })
})
