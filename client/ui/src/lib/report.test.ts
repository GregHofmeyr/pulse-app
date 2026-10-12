import { describe, expect, it } from 'vitest'
import { saveFlow, submitFlow } from './report'

describe('report flow', () => {
  it('sent: thanks with the short id', async () => {
    const steps: string[] = []
    const s = await submitFlow('voice cut out', { submit: async () => 'TVWXYZ', onStep: (st) => steps.push(st.step) })
    expect(steps).toEqual(['sending'])
    expect(s).toEqual({ step: 'sent', message: 'Thanks! Report #TVWXYZ sent.' })
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
    expect(s.message).toBe('Saved to C:\\Users\\sam\\Downloads\\pulse-report-20261012-132507.txt — send it to Greg on WhatsApp.')
  })

  it('save failure is reported, not swallowed', async () => {
    const s = await saveFlow('x', { save: async () => { throw 'permission denied' } })
    expect(s).toEqual({ step: 'failed', message: "Couldn't save the report: permission denied" })
  })
})
