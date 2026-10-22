import { describe, expect, it } from 'vitest'
import { addRtxTime } from './sdp'

const sdp = ['v=0', 'a=rtpmap:96 H264/90000', 'a=rtpmap:97 rtx/90000', 'a=fmtp:97 apt=96', 'a=fmtp:99 apt=98;rtx-time=200', ''].join('\r\n')

describe('addRtxTime', () => {
  it('adds rtx-time to RTX fmtp lines that lack it and leaves others alone', () => {
    const out = addRtxTime(sdp, 300)
    expect(out).toContain('a=fmtp:97 apt=96;rtx-time=300\r\n')
    expect(out).toContain('a=fmtp:99 apt=98;rtx-time=200\r\n')
    expect(out.split('\r\n').length).toBe(sdp.split('\r\n').length)
  })
})
