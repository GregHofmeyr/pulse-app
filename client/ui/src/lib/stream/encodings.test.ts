import { describe, expect, it } from 'vitest'
import { encodingFor, LOW_LAYER, MID_LAYER, resolutionsFor, simulcastLayersFor, type Fps, type Res } from './encodings'

describe('encodingFor', () => {
  it('matches the spec table and tunes games for motion', () => {
    expect(encodingFor(720, 60)).toEqual({ maxBitrate: 5_500_000, maxFramerate: 60, contentHint: 'motion', degradation: 'maintain-framerate' })
    expect(encodingFor(1080, 60).maxBitrate).toBe(8_000_000)
    expect(encodingFor(1440, 30).maxBitrate).toBe(7_500_000)
  })
  it('bitrates_match_the_spec_table', () => {
    const table: Record<Res, Record<Fps, number>> = {
      720: { 15: 1500, 30: 3500, 60: 5500 },
      1080: { 15: 2500, 30: 5000, 60: 8000 },
      1440: { 15: 4000, 30: 7500, 60: 12000 },
    }
    for (const res of [720, 1080, 1440] as Res[])
      for (const fps of [15, 30, 60] as Fps[]) expect(encodingFor(res, fps).maxBitrate).toBe(table[res][fps] * 1000)
  })
  it('fifteen_fps_is_detail', () => {
    expect(encodingFor(1080, 15)).toEqual({ maxBitrate: 2_500_000, maxFramerate: 15, contentHint: 'detail', degradation: 'maintain-resolution' })
    expect(encodingFor(720, 30).contentHint).toBe('motion')
  })
  it('has the always-on low layer', () => {
    expect(LOW_LAYER).toEqual({ width: 640, height: 360, maxBitrate: 500_000, maxFramerate: 30 })
  })
})

describe('simulcastLayersFor', () => {
  // previews pick the layer that fits their size on screen (#582): 1080p and up adds 720p between low and top
  it('a_720p_share_sends_low_under_its_top_layer_only', () => {
    expect(simulcastLayersFor(720, 30)).toEqual([LOW_LAYER])
    expect(simulcastLayersFor(720, 60)).toEqual([LOW_LAYER])
  })
  it('1080p_and_1440p_shares_add_a_720p_middle_layer', () => {
    for (const res of [1080, 1440] as Res[]) {
      expect(simulcastLayersFor(res, 30)).toEqual([LOW_LAYER, MID_LAYER])
      expect(simulcastLayersFor(res, 60)).toEqual([LOW_LAYER, MID_LAYER])
    }
    expect(MID_LAYER).toEqual({ width: 1280, height: 720, maxBitrate: 1_500_000, maxFramerate: 30 })
  })
  it('a_15_fps_share_spends_less_on_its_middle_layer', () => {
    expect(simulcastLayersFor(1080, 15)[1]).toEqual({ ...MID_LAYER, maxBitrate: 1_000_000 })
  })
  it('every_layer_stays_under_the_top_layers_bitrate', () => {
    for (const res of [720, 1080, 1440] as Res[])
      for (const fps of [15, 30, 60] as Fps[])
        for (const l of simulcastLayersFor(res, fps)) expect(l.maxBitrate).toBeLessThan(encodingFor(res, fps).maxBitrate)
  })
})

describe('resolutionsFor', () => {
  it('resolutions_never_exceed_the_source', () => {
    expect(resolutionsFor(1080)).toEqual([720, 1080])
    expect(resolutionsFor(1440)).toEqual([720, 1080, 1440])
    expect(resolutionsFor(2160)).toEqual([720, 1080, 1440])
    expect(resolutionsFor(720)).toEqual([720])
  })
  it('a source under 720 still offers 720', () => {
    expect(resolutionsFor(540)).toEqual([720])
  })
})
