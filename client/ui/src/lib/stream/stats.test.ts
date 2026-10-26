import { describe, expect, it } from 'vitest'
import { fpsNote, inboundSummary, isHardwareEncoder, outboundSummary } from './stats'

type S = Record<string, unknown> & { id: string; type: string }
const report = (...stats: S[]) => new Map(stats.map((s) => [s.id, s])) as unknown as RTCStatsReport

const out = (o: Partial<S> = {}): S => ({
  id: 'o1', type: 'outbound-rtp', kind: 'video', rid: 'f', frameWidth: 1280, frameHeight: 720,
  framesEncoded: 0, framesSent: 0, totalEncodeTime: 0, bytesSent: 0, qualityLimitationReason: 'none',
  encoderImplementation: 'NVENC', powerEfficientEncoder: true, ...o,
})

describe('outboundSummary', () => {
  it('encode_ms_is_per_interval_not_cumulative', () => {
    // 100 s of slow history, then 2 s of fast frames: cumulative would read ~10 ms, the interval is 2 ms.
    const prev = report(out({ framesEncoded: 1000, totalEncodeTime: 10 }))
    const cur = report(out({ framesEncoded: 1120, totalEncodeTime: 10.24, framesSent: 120, bytesSent: 250_000 }))
    const s = outboundSummary(prev, cur, 2000, 60)
    expect(s.layers[0].encodeMs).toBeCloseTo(2, 5)
    expect(s.layers[0].fps).toBe(60)
    expect(s.layers[0].kbps).toBe(1000)
  })
  it('has no encodeMs without a previous sample or new frames', () => {
    expect(outboundSummary(null, report(out({ framesEncoded: 5, totalEncodeTime: 1 })), 2000, 60).layers[0].encodeMs).toBeNull()
    const r = report(out({ framesEncoded: 5, totalEncodeTime: 1 }))
    expect(outboundSummary(r, r, 2000, 60).layers[0].encodeMs).toBeNull()
  })
  it('capture_fps_comes_from_media_source', () => {
    const cur = report(out(), { id: 'm', type: 'media-source', kind: 'video', framesPerSecond: 41 }, { id: 'r', type: 'remote-inbound-rtp', kind: 'video', roundTripTime: 0.025 })
    const s = outboundSummary(null, cur, 2000, 60)
    expect(s.captureFps).toBe(41)
    expect(s.rttMs).toBe(25)
  })
  it('captureFps and rtt are null when absent', () => {
    const s = outboundSummary(null, report(out()), 2000, 60)
    expect(s.captureFps).toBeNull()
    expect(s.rttMs).toBeNull()
  })
  it('judges the widest layer and reports every layer', () => {
    const lo = out({ id: 'o0', rid: 'q', frameWidth: 640, frameHeight: 360, encoderImplementation: 'OpenH264', powerEfficientEncoder: undefined })
    const s = outboundSummary(null, report(lo, out({ framesPerSecond: 60, framesSent: 0 })), 2000, 60)
    expect(s.layers).toHaveLength(2)
    expect(s.layers.find((l) => l.rid === 'q')?.powerEfficient).toBeNull()
    expect(s.note).toBe('ok')
  })
  it('still_screen_is_not_a_problem', () => {
    const cur = report(out({ framesSent: 4 }), { id: 'm', type: 'media-source', kind: 'video', framesPerSecond: 2 })
    expect(outboundSummary(report(out()), cur, 2000, 60).note).toBe('still')
  })
})

describe('fpsNote', () => {
  it('still_screen_is_not_a_problem', () => expect(fpsNote(2, 2, 60, 'none')).toBe('still'))
  it('game_capture_short_of_target_is_capture_limited', () => expect(fpsNote(40, 40, 60, 'none')).toBe('capture-limited'))
  it('cpu_limited_wins_over_capture', () => {
    expect(fpsNote(2, 2, 60, 'cpu')).toBe('encoder-limited')
    expect(fpsNote(40, 40, 60, 'cpu')).toBe('encoder-limited')
  })
  it('is ok at 85% of target, network-limited on bandwidth, encoder-limited when capture is fine', () => {
    expect(fpsNote(60, 51, 60, 'none')).toBe('ok')
    expect(fpsNote(60, 51, 60, 'cpu')).toBe('ok')
    expect(fpsNote(60, 30, 60, 'bandwidth')).toBe('network-limited')
    expect(fpsNote(60, 30, 60, 'none')).toBe('encoder-limited')
    expect(fpsNote(null, 30, 60, 'none')).toBe('encoder-limited')
  })
})

describe('inboundSummary', () => {
  const inb = (o: Partial<S> = {}): S => ({
    id: 'i', type: 'inbound-rtp', kind: 'video', frameWidth: 1920, frameHeight: 1080, framesDecoded: 0, bytesReceived: 0,
    freezeCount: 0, totalFreezesDuration: 0, packetsLost: 0, nackCount: 0, pliCount: 0,
    jitterBufferDelay: 0, jitterBufferEmittedCount: 0, decoderImplementation: 'D3D11', powerEfficientDecoder: true, ...o,
  })
  it('reports interval deltas', () => {
    const prev = report(inb({ framesDecoded: 100, bytesReceived: 1000, freezeCount: 3, totalFreezesDuration: 1, packetsLost: 5, nackCount: 2, pliCount: 1, jitterBufferDelay: 2, jitterBufferEmittedCount: 100 }))
    const cur = report(inb({ framesDecoded: 220, bytesReceived: 251_000, freezeCount: 4, totalFreezesDuration: 1.5, packetsLost: 8, nackCount: 5, pliCount: 2, jitterBufferDelay: 2.6, jitterBufferEmittedCount: 200 }))
    const s = inboundSummary(prev, cur, 2000)
    expect(s).toMatchObject({ fps: 60, w: 1920, h: 1080, kbps: 1000, freezes: 1, freezeMs: 500, lost: 3, nacks: 3, plis: 1, decoder: 'D3D11', powerEfficient: true, stillLikely: false })
    expect(s.jitterMsPerFrame).toBeCloseTo(6, 5)
  })
  it('inbound_still_is_flagged_not_counted_as_trouble', () => {
    const prev = report(inb({ framesDecoded: 10 }))
    const cur = report(inb({ framesDecoded: 14, freezeCount: 1, totalFreezesDuration: 1.8 }))
    const s = inboundSummary(prev, cur, 2000)
    expect(s.fps).toBe(2)
    expect(s.lost).toBe(0)
    expect(s.stillLikely).toBe(true)
  })
  it('is not stillLikely when packets were lost', () => {
    const s = inboundSummary(report(inb()), report(inb({ framesDecoded: 4, packetsLost: 2 })), 2000)
    expect(s.stillLikely).toBe(false)
  })
  it('without a previous sample deltas are zero', () => {
    const s = inboundSummary(null, report(inb({ framesDecoded: 50, freezeCount: 9, packetsLost: 4 })), 2000)
    expect(s).toMatchObject({ freezes: 0, lost: 0, kbps: 0, jitterMsPerFrame: null })
  })
})

describe('isHardwareEncoder', () => {
  it('nvidia_simulcast_adapter_is_hardware', () => {
    expect(isHardwareEncoder('SimulcastEncoderAdapter (MediaFoundationVideoEncodeAccelerator, MediaFoundationVideoEncodeAccelerator)', true)).toBe(true)
  })
  it('openh264_is_software', () => {
    expect(isHardwareEncoder('OpenH264', undefined)).toBe(false)
    expect(isHardwareEncoder('libvpx', undefined)).toBe(false)
    expect(isHardwareEncoder('ExternalEncoder', undefined)).toBe(true)
  })
  it('powerEfficient wins when defined', () => {
    expect(isHardwareEncoder('OpenH264', true)).toBe(true)
    expect(isHardwareEncoder('MediaFoundation', false)).toBe(false)
  })
})
