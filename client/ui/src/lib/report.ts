import { errorText } from './tauri'

/** What the page knows that Rust doesn't, sent with a bug report (#605). Only these fields: never a window title. */
export type ReportUi = {
  stream: {
    live: { height: number; fps: number; audio: boolean; sound: string | null; source: string | null; reconnecting: boolean } | null
    watching: number
    watching_ghost: number
  }
}

type LiveLike = { res: number; fps: number; audio: boolean; sound?: string; reconnecting?: boolean }

/** `surface`: the capture track's `displaySurface` (monitor / window / browser), if it says. */
export function reportUi(live: LiveLike | null, watching: { ghost?: boolean }[], surface: string | null | undefined): ReportUi {
  return {
    stream: {
      live: live && {
        height: live.res,
        fps: live.fps,
        audio: live.audio,
        sound: live.audio ? (live.sound ?? 'app') : null,
        source: surface ?? null,
        reconnecting: !!live.reconnecting,
      },
      watching: watching.length,
      watching_ghost: watching.filter((w) => w.ghost).length,
    },
  }
}

export type ReportStep = 'edit' | 'sending' | 'sent' | 'failed' | 'saving' | 'saved'
export type ReportState = { step: ReportStep; message: string }

/** The Rust side already retried once on a network blip; a failure here is final. */
export async function submitFlow(
  text: string,
  /** `noun`: what the thanks calls it ("Report", or "Idea" for #218). */
  deps: { submit: (t: string) => Promise<string>; onStep: (s: ReportState) => void; noun?: string },
): Promise<ReportState> {
  deps.onStep({ step: 'sending', message: '' })
  try {
    const short = await deps.submit(text)
    return { step: 'sent', message: `Thanks! ${deps.noun ?? 'Report'} #${short} sent.` }
  } catch (e) {
    const msg = errorText(e)
    const unreachable = msg.startsWith("can't reach the server") || msg.startsWith('the server is having trouble')
    return { step: 'failed', message: unreachable ? "Couldn't reach the server." : msg }
  }
}

export async function saveFlow(text: string, deps: { save: (t: string) => Promise<string> }): Promise<ReportState> {
  try {
    const path = await deps.save(text)
    return { step: 'saved', message: `Saved to ${path} — send it to Greg.` }
  } catch (e) {
    return { step: 'failed', message: `Couldn't save the report: ${errorText(e)}` }
  }
}
