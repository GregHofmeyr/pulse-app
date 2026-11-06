// Stream sound (#215): the sharer's choice, how it defaults, and what the live controls say about it.

/** Where stream audio comes from when it is on. */
export type Sound = 'app' | 'everything'
/** The Go Live choice: a source, or no sound at all (nothing published). */
export type SoundChoice = Sound | 'off'

/** The picked window, for finding its app: the capture source's window handle when the webview gives one. */
export type WindowRef = { hwnd: number | null; title: string }
/** Before going live: whether the window's app can be told apart, and whether it is playing now. */
export type SoundProbe = { identified: boolean; app: string | null; playing: boolean }
/** While live, from Rust: which source is heard (see `plan::Tier`), and whether anything has been since it began. */
export type SoundStatus = { tier: 'app' | 'related' | 'everything' | 'lastResort'; app: string | null; heard: boolean; ms: number }

export const CHOICE_LABEL: Record<SoundChoice, string> = { app: 'This app', everything: 'Everything except Pulse', off: 'No sound' }
export const UNIDENTIFIED_TIP = "Can't tell which app owns this window"
/** The last-resort tier, said in the controls. */
export const LAST_RESORT_TIP = "Couldn't tell which app owns this window: everything except Pulse, Discord and music apps"
/** How long This app may stay unheard before the controls say so. */
export const NO_SOUND_YET_MS = 20_000

/** A screen share has no app of its own. */
export const soundChoices = (surface: string): SoundChoice[] => (surface === 'window' ? ['app', 'everything', 'off'] : ['everything', 'off'])

/** The stored choice for this source: This app only for a window whose app is identified (`null`: not known yet). */
export function defaultChoice(stored: { audio: boolean; sound: Sound }, surface: string, identified: boolean | null): SoundChoice {
  if (!stored.audio) return 'off'
  if (surface !== 'window' || stored.sound === 'everything' || identified === false) return 'everything'
  return 'app'
}

/** Chromium's capture source id for a window is `window:<native handle>:<n>`; anything else has no handle. */
export function windowHandle(deviceId: string | undefined): number | null {
  const m = /^window:(\d+):/.exec(deviceId ?? '')
  const n = m ? Number(m[1]) : 0
  return Number.isSafeInteger(n) && n > 0 ? n : null
}

/** The live controls' line for the source in use ('' while it is starting). */
export function soundLabel(audio: boolean, s: SoundStatus | null): string {
  if (!audio) return 'No sound'
  if (!s) return ''
  const app = s.app ?? 'this app'
  switch (s.tier) {
    case 'app':
      return `Sound: ${app}`
    case 'related':
      return `Sound: ${app} and related apps`
    case 'everything':
      return 'Sound: everything except Pulse'
    case 'lastResort':
      return 'Sound: everything except Pulse and music apps'
  }
}

/** This app has been live a while with nothing heard: offer the switch (never made for them). */
export const showNoSoundYet = (s: SoundStatus | null): boolean =>
  !!s && (s.tier === 'app' || s.tier === 'related') && !s.heard && s.ms >= NO_SOUND_YET_MS
