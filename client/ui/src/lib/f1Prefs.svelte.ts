// The per-device "F1 race weekends" setting (#386), on its own so any card can read it without the F1 module. Also the
// F1 countdown strip's own switch and "hide for this weekend" (#456), both per device too.
const SETTING_KEY = 'pulse.f1'
const STRIP_KEY = 'pulse.f1.strip'
const HIDDEN_KEY = 'pulse.f1.hiddenWeekend'

/** A race weekend whose strip you hid: its key (season-round) and race name (for Settings to say which). */
export type HiddenWeekend = { key: string; name: string }

function read(key: string): string | null {
  try {
    return localStorage.getItem(key)
  } catch {
    return null
  }
}
function write(key: string, v: string | null): void {
  try {
    if (v === null) localStorage.removeItem(key)
    else localStorage.setItem(key, v)
  } catch {
    /* ignore */
  }
}
function savedHidden(): HiddenWeekend | null {
  try {
    const v = JSON.parse(read(HIDDEN_KEY) ?? 'null') as HiddenWeekend | null
    return v && typeof v.key === 'string' && typeof v.name === 'string' ? v : null
  } catch {
    return null
  }
}

/** "F1 race weekends" (this device only; on by default). Off: F1 cards fold to one line (click to open one) and the
 *  F1 channel has no countdown strip. Nothing changes for anyone else. `strip`: the countdown strip atop the F1
 *  channel on its own (on by default); `hidden`: the weekend whose strip was hidden until the next one. */
export const f1Prefs = $state({ on: read(SETTING_KEY) !== 'off', strip: read(STRIP_KEY) !== 'off', hidden: savedHidden() })

export function setF1RaceWeekends(v: boolean): void {
  f1Prefs.on = v
  write(SETTING_KEY, v ? 'on' : 'off')
}

/** The countdown strip's own switch (#456): off hides it in every F1 channel; F1 cards are untouched. */
export function setF1Strip(v: boolean): void {
  f1Prefs.strip = v
  write(STRIP_KEY, v ? 'on' : 'off')
}

/** A race weekend's key: one per season and round, so the next weekend's strip shows again by itself. */
export const weekendKey = (w: { season: number; round: number }): string => `${w.season}-${w.round}`

/** Hides the strip until the next race weekend (#456). */
export function hideF1Weekend(w: { season: number; round: number; race_name: string }): void {
  f1Prefs.hidden = { key: weekendKey(w), name: w.race_name }
  write(HIDDEN_KEY, JSON.stringify(f1Prefs.hidden))
}

/** Brings back a strip hidden for this weekend. */
export function showF1WeekendAgain(): void {
  f1Prefs.hidden = null
  write(HIDDEN_KEY, null)
}
