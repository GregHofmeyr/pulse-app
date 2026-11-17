// Settings › Chat › Clip drop (#382): the switch and the folders it watches, per device, on by default. Its own feature:
// it doesn't need Game activity (Settings › Privacy) and never turns game detection on. The core is told at boot and on
// every change (`set_clip_drop`), and offers new recordings (`pulse://clip-drop`, lib/clipDrop.svelte.ts).
//
// It used to live in Game activity's save (`pulse.games`: `clips` and `clip_folders`, under the master switch). A device
// with no Clip drop save yet takes what it had there: on only if both Game activity and its clips switch were on, so
// nobody's clip drop changes by itself.
import { api, type ClipDropSettings } from './tauri'

const KEY = 'pulse.clipDrop'
const GAMES_KEY = 'pulse.games'

export function defaults(): ClipDropSettings {
  return { on: true, folders: null }
}

const folderList = (v: unknown): string[] | null =>
  Array.isArray(v) ? v.filter((f): f is string => typeof f === 'string' && !!f.trim()) : null

/** What's saved, filled in with the defaults (junk is the defaults); with no save, what Game activity's save had. */
export function parseSaved(raw: string | null, gamesRaw: string | null): ClipDropSettings {
  const d = defaults()
  if (raw) {
    try {
      const v = JSON.parse(raw) as Partial<Record<keyof ClipDropSettings, unknown>>
      return { on: typeof v.on === 'boolean' ? v.on : d.on, folders: folderList(v.folders) }
    } catch {
      return d
    }
  }
  if (!gamesRaw) return d
  try {
    const g = JSON.parse(gamesRaw) as { enabled?: unknown; clips?: unknown; clip_folders?: unknown }
    // each defaulted on, as Game activity's save did
    return { on: g.enabled !== false && g.clips !== false, folders: folderList(g.clip_folders) }
  } catch {
    return d
  }
}

function read(key: string): string | null {
  try {
    return localStorage.getItem(key)
  } catch {
    return null
  }
}

function save(s: ClipDropSettings): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(s))
  } catch {
    /* ignore */
  }
}

function load(): ClipDropSettings {
  const raw = read(KEY)
  const s = parseSaved(raw, read(GAMES_KEY))
  // taken over from Game activity's save: kept as our own from now on, whatever Game activity does next
  if (!raw && read(GAMES_KEY)) save(s)
  return s
}

export const clipDropPrefs = $state<ClipDropSettings>(load())

function snapshot(): ClipDropSettings {
  return { on: clipDropPrefs.on, folders: clipDropPrefs.folders ? [...clipDropPrefs.folders] : null }
}

/** Tell the core the switch and folders (at boot: nothing is watched until then). */
export function tellClipDrop(): void {
  void api.setClipDrop(snapshot()).catch(() => {})
}

function saveAndTell(): void {
  save(snapshot())
  tellClipDrop()
}

export function setClipDropOn(on: boolean): void {
  clipDropPrefs.on = on
  saveAndTell()
}

/** The watched folders: `list` replaces the defaults (`null` goes back to them). */
export function setClipFolders(list: string[] | null): void {
  clipDropPrefs.folders = list ? [...new Set(list.filter((f) => !!f.trim()))] : null
  saveAndTell()
}
