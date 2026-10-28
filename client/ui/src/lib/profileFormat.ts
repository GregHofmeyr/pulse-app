/** Voice time: "N min" under an hour, one decimal under 10 h ("2.5 h"), then whole hours (plan reconciliation 16). */
export function formatVoice(seconds: number): string {
  const min = Math.round(seconds / 60)
  if (min < 60) return `${min} min`
  const h = seconds / 3600
  return h < 9.95 ? `${h.toFixed(1)} h` : `${Math.round(h)} h`
}

/** "Oct 3, 2026". */
export function formatMemberSince(iso: string): string {
  return new Date(iso).toLocaleDateString('en-US', { year: 'numeric', month: 'short', day: 'numeric', timeZone: 'UTC' })
}
