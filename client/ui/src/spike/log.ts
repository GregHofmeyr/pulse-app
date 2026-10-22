import { invoke } from '@tauri-apps/api/core'

/** One `spike:` JSON line in the app log (so every bug report carries it). */
export function spikeLog(obj: object) {
  void invoke('spike_log', { line: JSON.stringify({ ...obj, at: Date.now() }) }).catch(() => {})
}

/** Every test logs a line, pass or fail: silence would read as "not run". */
export async function logResult(test: string, fn: () => Promise<object>) {
  try {
    spikeLog({ test, ok: true, ...(await fn()) })
  } catch (e) {
    spikeLog({ test, ok: false, error: String(e) })
  }
}
