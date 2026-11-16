// Settings → Privacy → Game activity (#380): the master switch, "Show game art", "Show play time" and the games added
// by hand, per device (each profile has its own webview storage), all on by default. The master switch off means the
// core runs nothing and sends nothing. The core is told at boot and on every change, and tells back the game it sees
// (`pulse://game-activity`) for Settings to show. A hand-added game's exe path stays here and in the core: only its name
// is ever sent. Later switches (Join, achievements, clip drop) are one more key in `GameSettings`, defaulting on.
import { listen } from '@tauri-apps/api/event'
import { api, type GameSettings, type GameState, type ManualGame } from './tauri'

const KEY = 'pulse.games'

/** The switches a game activity group has; each defaults on. */
export type GameSwitch = 'enabled' | 'art' | 'time'

export function defaults(): GameSettings {
  return { enabled: true, art: true, time: true, games: [] }
}

/** What's saved, filled in with the defaults (a switch an older save lacks is on); junk is the defaults. */
export function parseSaved(raw: string | null): GameSettings {
  const d = defaults()
  if (!raw) return d
  try {
    const v = JSON.parse(raw) as Partial<GameSettings>
    const flag = (k: GameSwitch) => (typeof v[k] === 'boolean' ? (v[k] as boolean) : d[k])
    const games = Array.isArray(v.games)
      ? v.games.filter((g): g is ManualGame => !!g && typeof g.name === 'string' && typeof g.exe === 'string' && !!g.name.trim() && !!g.exe)
      : []
    return { enabled: flag('enabled'), art: flag('art'), time: flag('time'), games }
  } catch {
    return d
  }
}

function load(): GameSettings {
  try {
    return parseSaved(localStorage.getItem(KEY))
  } catch {
    return defaults()
  }
}

export const gamePrefs = $state<GameSettings>(load())

/** The game the core sees right now (Settings' "Pulse sees …"). */
export const gameActivity = $state<GameState>({ detecting: false, playing: null })

function snapshot(): GameSettings {
  return { enabled: gamePrefs.enabled, art: gamePrefs.art, time: gamePrefs.time, games: gamePrefs.games.map((g) => ({ ...g })) }
}

function saveAndTell(): void {
  const s = snapshot()
  try {
    localStorage.setItem(KEY, JSON.stringify(s))
  } catch {
    /* ignore */
  }
  void api.setGameActivity(s).catch(() => {})
}

export function setGameSwitch(k: GameSwitch, on: boolean): void {
  gamePrefs[k] = on
  saveAndTell()
}

/** Add a hand-picked game; one already listed (the same exe, any case) is renamed instead. */
export function addGame(name: string, exe: string): void {
  const n = name.trim()
  if (!n || !exe) return
  const i = gamePrefs.games.findIndex((g) => g.exe.toLowerCase() === exe.toLowerCase())
  if (i >= 0) gamePrefs.games[i] = { name: n, exe }
  else gamePrefs.games.push({ name: n, exe })
  saveAndTell()
}

export function removeGame(exe: string): void {
  gamePrefs.games = gamePrefs.games.filter((g) => g.exe !== exe)
  saveAndTell()
}

/** At boot: follow what the core sees, and tell it the saved switches (nothing runs until then). */
export async function startGameActivity(): Promise<() => void> {
  const off = await listen<GameState>('pulse://game-activity', (e) => {
    gameActivity.detecting = e.payload.detecting
    gameActivity.playing = e.payload.playing
  })
  void api.setGameActivity(snapshot()).catch(() => {})
  try {
    const now = await api.gameActivity()
    gameActivity.detecting = now.detecting
    gameActivity.playing = now.playing
  } catch {
    /* stays as it was */
  }
  return off
}
