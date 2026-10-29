/** Voice time: "N min" under an hour (never "60 min"), one decimal under 10 h ("2.5 h"), then whole
 * hours (plan reconciliation 16). */
export function formatVoice(seconds: number): string {
  if (seconds < 3600) return `${Math.min(59, Math.round(seconds / 60))} min`
  const h = seconds / 3600
  return h < 9.95 ? `${h.toFixed(1)} h` : `${Math.round(h)} h`
}

/** "Oct 3, 2026". */
export function formatMemberSince(iso: string): string {
  return new Date(iso).toLocaleDateString('en-US', { year: 'numeric', month: 'short', day: 'numeric', timeZone: 'UTC' })
}
