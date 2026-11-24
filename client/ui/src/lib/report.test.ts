import { describe, expect, it } from 'vitest'
import { reportUi, saveFlow, submitFlow } from './report'

describe('report flow', () => {
  it('sent: thanks with the short id', async () => {
    const steps: string[] = []
    const s = await submitFlow('voice cut out', { submit: async () => 'TVWXYZ', onStep: (st) => steps.push(st.step) })
    expect(steps).toEqual(['sending'])
    expect(s).toEqual({ step: 'sent', message: 'Thanks! Report #TVWXYZ sent.' })
  })

  it('an idea is thanked as one (#218)', async () => {
    const s = await submitFlow('pin clips', { submit: async () => 'TVWXYZ', onStep: () => {}, noun: 'Idea' })
    expect(s).toEqual({ step: 'sent', message: 'Thanks! Idea #TVWXYZ sent.' })
  })

  it('unreachable: offers saving', async () => {
    const s = await submitFlow('x', { submit: async () => { throw "can't reach the server: dns error" }, onStep: () => {} })
    expect(s).toEqual({ step: 'failed', message: "Couldn't reach the server." })
  })

  it('refused: shows the server reason', async () => {
    const s = await submitFlow('x', { submit: async () => { throw 'slow down a little, then try again' }, onStep: () => {} })
    expect(s).toEqual({ step: 'failed', message: 'slow down a little, then try again' })
  })

  it('saved: says where and what to do', async () => {
    const s = await saveFlow('x', { save: async () => 'C:\\Users\\sam\\Downloads\\pulse-report-20261012-132507.txt' })
    expect(s.step).toBe('saved')
    expect(s.message).toBe('Saved to C:\\Users\\sam\\Downloads\\pulse-report-20261012-132507.txt — send it to Greg.')
  })

  it('save failure is reported, not swallowed', async () => {
    const s = await saveFlow('x', { save: async () => { throw 'permission denied' } })
    expect(s).toEqual({ step: 'failed', message: "Couldn't save the report: permission denied" })
  })

  it('the stream state for a report (#605): sizes and kinds only, never the window title', () => {
    const live = { channelId: 'c1', res: 1080, fps: 60, audio: true, sound: 'everything', label: 'Secret window', reconnecting: false }
    const ui = reportUi(live, [{ ghost: false }, { ghost: true }], 'window')
    expect(ui).toEqual({
      stream: {
        live: { height: 1080, fps: 60, audio: true, sound: 'everything', source: 'window', reconnecting: false },
        watching: 2,
        watching_ghost: 1,
      },
    })
    expect(JSON.stringify(ui)).not.toContain('Secret')
    expect(reportUi({ res: 720, fps: 30, audio: false }, [], undefined).stream.live).toEqual({
      height: 720, fps: 30, audio: false, sound: null, source: null, reconnecting: false,
    })
    expect(reportUi(null, [], null)).toEqual({ stream: { live: null, watching: 0, watching_ghost: 0 } })
  })
})
