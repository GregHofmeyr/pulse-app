// Settings › Chat › Clip drop (#382): the switch and the folders it watches, per device, on by default. Its own feature:
// it doesn't need Game activity (Settings › Privacy) and never turns game detection on. The core is told at boot and on
// every change (`set_clip_drop`), and offers new recordings (`pulse://clip-drop`, lib/clipDrop.svelte.ts).
// Old `clips`/`clip_folders` keys left in Game activity's save (`pulse.games`, before 0.2.17) are ignored.
import { api, type ClipDropSettings } from './tauri'

const KEY = 'pulse.clipDrop'

export function defaults(): ClipDropSettings {
  return { on: true, folders: null }
}

const folderList = (v: unknown): string[] | null =>
  Array.isArray(v) ? v.filter((f): f is string => typeof f === 'string' && !!f.trim()) : null

/** What's saved, filled in with the defaults (no save or junk is the defaults). */
export function parseSaved(raw: string | null): ClipDropSettings {
  const d = defaults()
  if (!raw) return d
  try {
    const v = JSON.parse(raw) as Partial<Record<keyof ClipDropSettings, unknown>>
    return { on: typeof v.on === 'boolean' ? v.on : d.on, folders: folderList(v.folders) }
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

export const clipDropPrefs = $state<ClipDropSettings>(parseSaved(read(KEY)))

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
