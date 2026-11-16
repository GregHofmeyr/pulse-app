// The per-device "F1 race weekends" setting (#386), on its own so any card can read it without the F1 module.
const SETTING_KEY = 'pulse.f1'

function savedOn(): boolean {
  try {
    return localStorage.getItem(SETTING_KEY) !== 'off'
  } catch {
    return true
  }
}

/** "F1 race weekends" (this device only; on by default). Off: F1 cards fold to one line (click to open one) and the
 *  F1 channel has no countdown banner. Nothing changes for anyone else. */
export const f1Prefs = $state({ on: savedOn() })

export function setF1RaceWeekends(v: boolean): void {
  f1Prefs.on = v
  try {
    localStorage.setItem(SETTING_KEY, v ? 'on' : 'off')
  } catch {
    /* ignore */
  }
}
