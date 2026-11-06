import { describe, expect, it } from 'vitest'
import { CHOICE_LABEL, defaultChoice, NO_SOUND_YET_MS, showNoSoundYet, soundChoices, soundLabel, windowHandle, type SoundStatus } from './sound'

const status = (s: Partial<SoundStatus>): SoundStatus => ({ tier: 'app', app: 'Firefox', heard: false, ms: 0, ...s })

describe('stream sound choice', () => {
  it('a_window_share_offers_three_choices_a_screen_two', () => {
    expect(soundChoices('window').map((c) => CHOICE_LABEL[c])).toEqual(['This app', 'Everything except Pulse', 'No sound'])
    expect(soundChoices('monitor').map((c) => CHOICE_LABEL[c])).toEqual(['Everything except Pulse', 'No sound'])
  })

  it('defaults_to_this_app_when_identified_else_everything', () => {
    expect(defaultChoice({ audio: true, sound: 'app' }, 'window', true)).toBe('app')
    expect(defaultChoice({ audio: true, sound: 'app' }, 'window', null)).toBe('app') // not known yet
    expect(defaultChoice({ audio: true, sound: 'app' }, 'window', false)).toBe('everything')
    expect(defaultChoice({ audio: true, sound: 'everything' }, 'window', true)).toBe('everything')
    expect(defaultChoice({ audio: true, sound: 'app' }, 'monitor', true)).toBe('everything')
    expect(defaultChoice({ audio: false, sound: 'app' }, 'window', true)).toBe('off')
  })

  it('reads_the_window_handle_from_the_capture_source', () => {
    expect(windowHandle('window:1312:0')).toBe(1312)
    expect(windowHandle('screen:0:0')).toBeNull()
    expect(windowHandle('window:abc:0')).toBeNull()
    expect(windowHandle('')).toBeNull()
    expect(windowHandle(undefined)).toBeNull()
    expect(windowHandle('window:0:0')).toBeNull()
  })

  it('labels_the_source_in_use', () => {
    expect(soundLabel(false, null)).toBe('No sound')
    expect(soundLabel(true, null)).toBe('')
    expect(soundLabel(true, status({}))).toBe('Sound: Firefox')
    expect(soundLabel(true, status({ app: null }))).toBe('Sound: this app')
    expect(soundLabel(true, status({ tier: 'related' }))).toBe('Sound: Firefox and related apps')
    expect(soundLabel(true, status({ tier: 'everything', app: null }))).toBe('Sound: everything except Pulse')
    expect(soundLabel(true, status({ tier: 'lastResort', app: null }))).toBe('Sound: everything except Pulse and music apps')
  })

  it('says_no_sound_yet_only_for_this_app_after_a_while_unheard', () => {
    expect(showNoSoundYet(null)).toBe(false)
    expect(showNoSoundYet(status({ ms: NO_SOUND_YET_MS - 1 }))).toBe(false)
    expect(showNoSoundYet(status({ ms: NO_SOUND_YET_MS }))).toBe(true)
    expect(showNoSoundYet(status({ tier: 'related', ms: NO_SOUND_YET_MS }))).toBe(true)
    expect(showNoSoundYet(status({ ms: NO_SOUND_YET_MS, heard: true }))).toBe(false)
    expect(showNoSoundYet(status({ tier: 'everything', ms: NO_SOUND_YET_MS }))).toBe(false)
    expect(showNoSoundYet(status({ tier: 'lastResort', ms: NO_SOUND_YET_MS }))).toBe(false)
    expect(NO_SOUND_YET_MS).toBe(20_000)
  })
})
