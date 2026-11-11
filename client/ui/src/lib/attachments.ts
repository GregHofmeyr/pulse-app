// Files in messages (#57): limits, sizes, the run-programs warning and the caption a message shows.
import type { Message } from './protocol/Message'
import type { Attachment } from './protocol/Attachment'

/** Files per message **(owner)**. */
export const MAX_FILES = 10
/** The most a normal file can be **(owner)**; bigger ones are temporary sends. */
export const MAX_BYTES = 50 * 1024 * 1024

export const TOO_MANY = `Up to ${MAX_FILES} files per message.`
export const TOO_BIG = 'Files can be up to 50 MB.'

/** "820 KB", "1.4 MB", "2.1 GB" (powers of 1024, as the limits are). */
export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  const units = ['KB', 'MB', 'GB']
  let v = bytes / 1024
  let u = 0
  while (v >= 1024 && u < units.length - 1) {
    v /= 1024
    u++
  }
  return `${v >= 10 || u === 0 ? Math.round(v) : v.toFixed(1)} ${units[u]}`
}

// Files Windows, macOS or Linux can run (or that run something) when opened.
const EXECUTABLE = new Set([
  'exe', 'msi', 'msix', 'msp', 'appx', 'bat', 'cmd', 'com', 'ps1', 'psm1', 'vbs', 'vbe', 'js', 'jse', 'wsf', 'wsh', 'scr',
  'pif', 'cpl', 'msc', 'reg', 'lnk', 'hta', 'jar', 'sh', 'bash', 'zsh', 'ksh', 'csh', 'fish', 'run', 'bin', 'appimage',
  'deb', 'rpm', 'apk', 'app', 'dmg', 'pkg', 'command', 'dll', 'sys', 'ocx', 'desktop', 'iso', 'img', 'vhd', 'vhdx',
  'chm', 'scf', 'url', 'inf', 'gadget', 'settingcontent-ms', 'xll', 'py', 'pyw', 'pl', 'rb', 'html', 'htm', 'xhtml',
  'svg', 'docm', 'xlsm', 'pptm',
])

/** Whether opening the file could run a program (the Download warning). By extension, case-insensitive. */
export function isExecutable(name: string): boolean {
  // Windows drops trailing dots and spaces: "setup.exe." saves as setup.exe
  const n = name.replace(/[.\s]+$/, '')
  const dot = n.lastIndexOf('.')
  return dot > 0 && EXECUTABLE.has(n.slice(dot + 1).toLowerCase())
}

export const RUNS_PROGRAMS = 'This file can run programs on your computer'

/** What a message shows as its text: nothing when its content is only the plain-text stand-in older
 *  versions of Pulse show for files with no caption. */
export function shownCaption(m: Pick<Message, 'content' | 'caption_is_fallback'>): string {
  return m.caption_is_fallback ? '' : m.content
}

/** A message's files split the way they show: images in the grid, video and audio players, file cards. */
export function arrange(atts: readonly Attachment[]): { images: Attachment[]; videos: Attachment[]; audio: Attachment[]; files: Attachment[] } {
  const r = { images: [] as Attachment[], videos: [] as Attachment[], audio: [] as Attachment[], files: [] as Attachment[] }
  for (const a of atts) {
    if (a.expires_at) r.files.push(a) // a temporary send is always a card (it expires)
    else if (a.kind === 'image') r.images.push(a)
    else if (a.kind === 'video') r.videos.push(a)
    else if (a.kind === 'audio') r.audio.push(a)
    else r.files.push(a)
  }
  return r
}

/** Grid columns for `n` images: one big, two side by side, three across from five. */
export function gridColumns(n: number): number {
  return n <= 1 ? 1 : n >= 5 ? 3 : 2
}

/** The box one image is shown in (no layout shift: the size is known before it loads), at most 400×300. */
export function singleBox(w: number | null, h: number | null): { width: number; height: number } {
  if (!w || !h) return { width: 400, height: 300 }
  const scale = Math.min(1, 400 / w, 300 / h)
  return { width: Math.max(1, Math.round(w * scale)), height: Math.max(1, Math.round(h * scale)) }
}

/** The most a temporary send can be **(owner)**. */
export const TEMP_MAX_BYTES = 2 * 1024 * 1024 * 1024
export const TOO_BIG_TEMP = 'Files can be up to 2 GB.'
/** How long a temporary send lasts: the sender picks **(owner)**. */
export const TEMP_DAYS = [1, 7] as const

/** Whether a temporary send is gone: the server says so, or its time is up. */
export function isExpired(a: Pick<Attachment, 'expires_at' | 'expired'>, nowMs: number): boolean {
  return a.expired || (!!a.expires_at && Date.parse(a.expires_at) <= nowMs)
}

/** "expires in 6 days", "expires tomorrow", "expires in 5 hours", "expires in 20 minutes". */
export function expiresLabel(expiresAt: string, nowMs: number): string {
  const left = Date.parse(expiresAt) - nowMs
  const minutes = Math.max(1, Math.ceil(left / 60_000))
  if (minutes < 60) return `expires in ${minutes} minute${minutes === 1 ? '' : 's'}`
  const hours = Math.round(left / 3_600_000)
  if (hours < 24) return `expires in ${hours} hour${hours === 1 ? '' : 's'}`
  const days = Math.round(left / 86_400_000)
  return days <= 1 ? 'expires tomorrow' : `expires in ${days} days`
}

/** A video that isn't a clip (a message's file) for the clip player: its own link, size and length. */
export type VideoSource = {
  id: string
  link: (fresh: boolean) => Promise<string>
  width: number | null
  height: number | null
  durationMs: number | null
}
