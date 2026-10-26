import { afterEach, describe, expect, it } from 'vitest'
import { addRtxTime, installRtxTime } from './sdp'

const sdp = ['v=0', 'a=rtpmap:96 H264/90000', 'a=rtpmap:97 rtx/90000', 'a=fmtp:97 apt=96', 'a=fmtp:99 apt=98;rtx-time=200', ''].join('\r\n')

describe('addRtxTime', () => {
  it('adds rtx-time to RTX fmtp lines that lack it and leaves others alone', () => {
    const out = addRtxTime(sdp, 300)
    expect(out).toContain('a=fmtp:97 apt=96;rtx-time=300\r\n')
    expect(out).toContain('a=fmtp:99 apt=98;rtx-time=200\r\n')
    expect(out.split('\r\n').length).toBe(sdp.split('\r\n').length)
  })
})

describe('installRtxTime', () => {
  const RTC = globalThis as unknown as { RTCPeerConnection?: unknown }
  const original = RTC.RTCPeerConnection
  afterEach(() => {
    RTC.RTCPeerConnection = original
  })

  function fakePc() {
    const seen: (RTCSessionDescriptionInit | undefined)[] = []
    class Fake {
      async setLocalDescription(d?: RTCSessionDescriptionInit) {
        seen.push(d)
      }
    }
    RTC.RTCPeerConnection = Fake
    return { Fake, seen }
  }

  it('rewrites answers with rtx-time=300, leaves offers alone, and patches only once', async () => {
    const { Fake, seen } = fakePc()
    installRtxTime()
    const patched = Fake.prototype.setLocalDescription
    installRtxTime()
    expect(Fake.prototype.setLocalDescription).toBe(patched)
    const pc = new Fake()
    await pc.setLocalDescription({ type: 'answer', sdp })
    await pc.setLocalDescription({ type: 'offer', sdp })
    expect(seen[0]?.sdp).toContain('a=fmtp:97 apt=96;rtx-time=300')
    expect(seen[1]?.sdp).toBe(sdp)
  })
})
