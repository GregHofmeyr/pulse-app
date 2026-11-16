// Inline previews of small text files (#318): which files get one, and the collapsed view's first lines. The core reads
// the file (attachment_text: UTF-8 only, never binary, at most TEXT_PREVIEW_MAX bytes); Markdown renders through the
// app's safe renderer (markdown.ts), everything else (HTML included) shows as plain source text, never as markup.
import type { Attachment } from './protocol/Attachment'
import { isExpired } from './attachments'

/** The largest file previewed (the core's `TEXT_PREVIEW_MAX`): bigger ones only download. */
export const TEXT_PREVIEW_MAX = 256 * 1024
/** Lines the collapsed preview shows. */
export const PREVIEW_LINES = 15

const MARKDOWN = new Set(['md', 'markdown'])
// Plain text, and source files shown as monospace text. HTML and SVG are here on purpose: they show as source.
const TEXT = new Set([
  'txt', 'log', 'csv', 'tsv', 'json', 'jsonc', 'xml', 'html', 'htm', 'svg', 'yml', 'yaml', 'toml', 'ini', 'cfg', 'conf', 'env',
  'js', 'mjs', 'cjs', 'ts', 'tsx', 'jsx', 'svelte', 'vue', 'css', 'scss', 'py', 'rs', 'go', 'java', 'kt', 'c', 'h', 'cpp', 'hpp',
  'cs', 'rb', 'php', 'lua', 'sh', 'bash', 'ps1', 'bat', 'sql', 'diff', 'patch', 'srt', 'vtt',
])

/** How `a` previews: rendered Markdown, plain text, or not at all (not a small text file, or expired). */
export function previewKind(a: Attachment, nowMs: number): 'markdown' | 'text' | null {
  if (a.kind !== 'file' || a.size > TEXT_PREVIEW_MAX || isExpired(a, nowMs)) return null
  const dot = a.name.lastIndexOf('.')
  const ext = dot > 0 ? a.name.slice(dot + 1).toLowerCase() : ''
  if (MARKDOWN.has(ext)) return 'markdown'
  if (TEXT.has(ext)) return 'text'
  return null
}

/** The collapsed view: the first `n` lines, and how many lines there are in all. */
export function firstLines(text: string, n = PREVIEW_LINES): { head: string; lines: number; more: boolean } {
  const all = text.split('\n')
  // a final newline doesn't make one more line
  const lines = all.length > 1 && all[all.length - 1] === '' ? all.length - 1 : all.length
  return { head: all.slice(0, n).join('\n'), lines, more: lines > n }
}
