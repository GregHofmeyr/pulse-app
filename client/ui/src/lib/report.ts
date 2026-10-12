import { errorText } from './tauri'

export type ReportStep = 'edit' | 'sending' | 'sent' | 'failed' | 'saving' | 'saved'
export type ReportState = { step: ReportStep; message: string }

/** The Rust side already retried once on a network blip; a failure here is final. */
export async function submitFlow(
  text: string,
  deps: { submit: (t: string) => Promise<string>; onStep: (s: ReportState) => void },
): Promise<ReportState> {
  deps.onStep({ step: 'sending', message: '' })
  try {
    const short = await deps.submit(text)
    return { step: 'sent', message: `Thanks! Report #${short} sent.` }
  } catch (e) {
    const msg = errorText(e)
    const unreachable = msg.startsWith("can't reach the server") || msg.startsWith('the server is having trouble')
    return { step: 'failed', message: unreachable ? "Couldn't reach the server." : msg }
  }
}

export async function saveFlow(text: string, deps: { save: (t: string) => Promise<string> }): Promise<ReportState> {
  try {
    const path = await deps.save(text)
    return { step: 'saved', message: `Saved to ${path} — send it to Greg on WhatsApp.` }
  } catch (e) {
    return { step: 'failed', message: `Couldn't save the report: ${errorText(e)}` }
  }
}
