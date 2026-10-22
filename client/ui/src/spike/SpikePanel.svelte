<script lang="ts">
  // THROWAWAY (screen-share spike): the hidden debug panel (Ctrl+Shift+F12). One section per test; every
  // button logs a `spike:` line, pass or fail, so a bug report carries the result.
  import { onDestroy } from 'svelte'
  import { invoke } from '@tauri-apps/api/core'
  import { listen } from '@tauri-apps/api/event'
  import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window'
  import { WebviewWindow } from '@tauri-apps/api/webviewWindow'
  import { VideoQuality, type LocalTrackPublication, type RemoteTrack, type Room } from 'livekit-client'
  import { voice } from '../lib/voice.svelte'
  import { logResult, spikeLog } from './log'
  import { connect, firstStreamer, pick, publish, watch, type PublishOpts } from './rooms'
  import { Ring, strain, type Sample } from './sampler'
  import type { Fps, Res } from './encodings'

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  type Stat = Record<string, any>

  const channel = $derived(voice.channelId)
  const need = () => {
    if (!channel) throw new Error('join a voice channel first')
    return channel
  }

  // ---- S0 setup
  let gpu = $state('')

  // ---- S1/S5 encode
  let opts = $state<PublishOpts>({ res: 1080, fps: 60, lowLayer: true, codec: 'h264' })
  let pubRoom: Room | null = null
  let pub = $state<LocalTrackPublication | null>(null)
  let mark = $state('')
  let prevOut: Sample | null = null
  let prevBytes = new Map<string, number>()
  let lastS1: object = {}

  async function goLive() {
    await stopLive()
    pubRoom = await connect(need(), 'v', false)
    pub = await publish(pubRoom, opts)
    return { ...opts }
  }

  async function stopLive() {
    if (pub?.track) await pubRoom?.localParticipant.unpublishTrack(pub.track)
    pub = null
    await pubRoom?.disconnect()
    pubRoom = null
    prevOut = null
    prevBytes = new Map()
    return {}
  }

  async function sampleOut() {
    const rep = await pub?.track?.getRTCStatsReport()
    if (!rep) return
    const all = [...rep.values()] as Stat[]
    const codecs = new Map(all.filter((s) => s.type === 'codec').map((s) => [s.id, s]))
    const layers = all
      .filter((s) => s.type === 'outbound-rtp' && s.kind === 'video')
      .map((s) => {
        const bytes = s.bytesSent - (prevBytes.get(s.id) ?? s.bytesSent)
        prevBytes.set(s.id, s.bytesSent)
        return {
          rid: s.rid,
          w: s.frameWidth,
          h: s.frameHeight,
          encoderImplementation: s.encoderImplementation,
          powerEfficientEncoder: s.powerEfficientEncoder,
          framesPerSecond: s.framesPerSecond,
          encodeMsPerFrame: s.framesEncoded ? (s.totalEncodeTime * 1000) / s.framesEncoded : null,
          qualityLimitationReason: s.qualityLimitationReason,
          sdpFmtpLine: codecs.get(s.codecId)?.sdpFmtpLine,
          mimeType: codecs.get(s.codecId)?.mimeType,
          targetBitrate: s.targetBitrate,
          kbps: Math.round((bytes * 8) / 2000),
          framesEncoded: s.framesEncoded,
          totalEncodeTime: s.totalEncodeTime,
          framesSent: s.framesSent,
        }
      })
    // the full layer: the widest encoding
    const full = layers.reduce<(typeof layers)[number] | null>((a, b) => (!a || (b.w ?? 0) > (a.w ?? 0) ? b : a), null)
    let st = null
    if (full) {
      const cur: Sample = {
        t: Date.now(),
        framesEncoded: full.framesEncoded ?? 0,
        totalEncodeTime: full.totalEncodeTime ?? 0,
        framesSent: full.framesSent ?? 0,
        qualityLimitationReason: full.qualityLimitationReason ?? 'none',
      }
      if (prevOut) st = strain(prevOut, cur, opts.fps)
      prevOut = cur
    }
    lastS1 = { layers: layers.length, full: full && { enc: full.encoderImplementation, fps: full.framesPerSecond, kbps: full.kbps } }
    spikeLog({ test: 'S1', event: 'outbound', layers })
    if (st) spikeLog({ test: 'S21', event: 'strain', ...st })
  }

  // ---- S4/S7 viewer
  let viewRoom: Room | null = null
  let remote = $state<RemoteTrack | null>(null)
  let streamer = $state('')
  let rtxTime = $state(false)
  let dataSaver = $state(false)
  let videoEl = $state<HTMLVideoElement | undefined>()
  let prevIn = new Map<string, Stat>()
  let lastS4: object = {}

  async function startWatch() {
    await stopWatch()
    viewRoom = await connect(need(), 'v', rtxTime)
    const who = streamer.trim() || firstStreamer(viewRoom)
    if (!who) throw new Error('nobody is live in this channel')
    remote = await watch(viewRoom, who, videoEl!)
    countFrames()
    return { streamer: who, rtxTime }
  }

  async function stopWatch() {
    remote?.detach()
    remote = null
    await viewRoom?.disconnect()
    viewRoom = null
    prevIn = new Map()
    return {}
  }

  async function setDataSaver(on: boolean) {
    for (const p of viewRoom?.remoteParticipants.values() ?? []) {
      for (const tp of p.trackPublications.values()) if (tp.kind === 'video') tp.setVideoQuality(on ? VideoQuality.LOW : VideoQuality.HIGH)
    }
    return { dataSaver: on }
  }

  async function sampleIn() {
    const rep = await remote?.getRTCStatsReport()
    if (!rep) return
    for (const s of rep.values() as Iterable<Stat>) {
      if (s.type !== 'inbound-rtp' || s.kind !== 'video') continue
      const p = prevIn.get(s.id)
      prevIn.set(s.id, s)
      const d = (k: string) => (p ? s[k] - p[k] : null)
      const line = {
        framesPerSecond: s.framesPerSecond,
        freezeCount: s.freezeCount,
        totalFreezesDuration: s.totalFreezesDuration,
        frameWidth: s.frameWidth,
        frameHeight: s.frameHeight,
        decoderImplementation: s.decoderImplementation,
        powerEfficientDecoder: s.powerEfficientDecoder,
        jitterBufferMsPerFrame: p && d('jitterBufferEmittedCount') ? (d('jitterBufferDelay')! * 1000) / d('jitterBufferEmittedCount')! : null,
        packetsLostDelta: d('packetsLost'),
        nackCount: s.nackCount,
        pliCount: s.pliCount,
      }
      lastS4 = { fps: line.framesPerSecond, freezes: line.freezeCount, w: line.frameWidth }
      spikeLog({ test: 'S4', event: 'inbound', rtxTime, dataSaver, ...line })
    }
  }

  // rVFC frame counting on the viewer <video> (S6 fps, S18 last frame)
  let rvfcFrames = 0
  let lastFrameAt = 0
  function countFrames() {
    const el = videoEl
    if (!el) return
    const tick = () => {
      rvfcFrames++
      lastFrameAt = Date.now()
      if (remote) el.requestVideoFrameCallback(tick)
    }
    el.requestVideoFrameCallback(tick)
  }

  // ---- S6 background
  let bgOn = $state(false)
  let bgPrevFrames = 0

  // ---- S7 decode stats unlock
  async function unlockStats() {
    const s = await navigator.mediaDevices.getUserMedia({ audio: true })
    for (const t of s.getTracks()) t.stop()
    return { unlocked: true }
  }

  // ---- S3 soak
  let soakOn = $state(false)
  const soakRing = new Ring<object>(120)
  let soakTimer: ReturnType<typeof setInterval> | undefined

  async function soakSample() {
    const os = await invoke<object>('spike_os_stats')
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const mem = (performance as any).memory
    const line = {
      os,
      jsHeapMb: mem ? Math.round(mem.usedJSHeapSize / 1048576) : null,
      s1: lastS1,
      s4: lastS4,
    }
    soakRing.push(line)
    spikeLog({ test: 'S3', event: 'sample', n: soakRing.items().length, ...line })
  }

  function toggleSoak() {
    soakOn = !soakOn
    clearInterval(soakTimer)
    if (soakOn) {
      void soakSample()
      soakTimer = setInterval(() => void soakSample(), 30_000)
    }
    spikeLog({ test: 'S3', event: soakOn ? 'soak_start' : 'soak_stop' })
  }

  async function cycle(kind: 'replace' | 'republish') {
    const room = pubRoom
    if (!room || !pub?.track) throw new Error('go live first')
    const results = []
    for (let i = 0; i < 20; i++) {
      const old = pub?.track
      if (!old) throw new Error(`share ended at cycle ${i}`)
      if (kind === 'replace') {
        const { video } = await pick(opts)
        const prev = old.mediaStreamTrack
        // the published track adopts the new capture; only the previous capture is stopped
        await old.replaceTrack(video.mediaStreamTrack, false)
        prev.stop()
      } else {
        await room.localParticipant.unpublishTrack(old)
        pub = await publish(room, opts)
      }
      const os = await invoke<object>('spike_os_stats')
      spikeLog({ test: 'S3', event: 'cycle', kind, i, os })
      results.push(i)
    }
    return { kind, cycles: results.length }
  }

  // ---- S8 picker
  let pickTitle = $state('')
  async function setHeight(h: number) {
    await getCurrentWindow().setSize(new LogicalSize(900, h))
    return { height: h }
  }
  async function pickOnly() {
    const { video, info } = await pick(opts)
    video.stop()
    pickTitle = info.label
    return { event: 'picked', ...info }
  }
  async function findPid() {
    const pid = await invoke<number | null>('spike_window_pid', { title: pickTitle })
    if (pid != null) audioPid = String(pid)
    return { title: pickTitle, pid }
  }

  // ---- S9/S19 audio
  let audioMode = $state<'include' | 'exclude'>('exclude')
  let audioPid = $state('')
  let tone = $state<'none' | 'left'>('none')
  async function startAudio() {
    const pid = audioPid.trim() ? Number(audioPid) : null
    await invoke('spike_loopback_start', { channelId: need(), mode: audioMode, pid, tone: tone === 'left' ? 'left' : null })
    return { mode: audioMode, pid, tone }
  }
  const stopAudio = async () => (await invoke('spike_loopback_stop'), {})
  const listenAudio = async () => (await invoke('spike_beep_watch', { channelId: need() }), {})

  let measuring = $state(false)
  async function measureAv() {
    const el = videoEl
    if (!el || !remote) throw new Error('watch a stream first')
    measuring = true
    const c = document.createElement('canvas')
    c.width = c.height = 16
    const g = c.getContext('2d', { willReadFrequently: true })!
    let dark = true
    const onFrame = (_now: number, meta: VideoFrameCallbackMetadata) => {
      if (!measuring || !remote) return
      const sx = Math.max(0, el.videoWidth / 2 - 8)
      const sy = Math.max(0, el.videoHeight / 2 - 8)
      g.drawImage(el, sx, sy, 16, 16, 0, 0, 16, 16)
      const px = g.getImageData(0, 0, 16, 16).data
      let sum = 0
      for (let i = 0; i < px.length; i += 4) sum += (px[i] + px[i + 1] + px[i + 2]) / 3
      const luma = sum / 256
      if (dark && luma > 200) {
        // performance clock → wall clock, so it pairs with the Rust side's beep t_ms
        const tMs = Math.round(Date.now() - (performance.now() - meta.expectedDisplayTime))
        spikeLog({ test: 'S9', event: 'flash', t_ms: tMs })
        dark = false
      } else if (!dark && luma < 30) dark = true
      el.requestVideoFrameCallback(onFrame)
    }
    el.requestVideoFrameCallback(onFrame)
    return { measuring: true }
  }

  // ---- S12 lifecycle
  function killRenderer() {
    spikeLog({ test: 'S12', event: 'kill_renderer' })
    location.href = 'edge://kill'
  }
  async function reconnectTest() {
    const times = []
    for (let i = 0; i < 5; i++) {
      const t0 = performance.now()
      const r = await connect(need(), 'v2', false)
      await r.disconnect()
      times.push(Math.round(performance.now() - t0))
    }
    return { connectDisconnectMs: times }
  }

  // ---- S13 preview grab
  let previewOn = $state(false)
  async function grabPreview() {
    const t = pub?.track?.mediaStreamTrack
    if (!t) return
    const t0 = performance.now()
    try {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const bmp: ImageBitmap = await new (window as any).ImageCapture(t).grabFrame()
      const oc = new OffscreenCanvas(480, 270)
      oc.getContext('2d')!.drawImage(bmp, 0, 0, 480, 270)
      bmp.close()
      const blob = await oc.convertToBlob({ type: 'image/webp', quality: 0.7 })
      spikeLog({ test: 'S13', ok: true, bytes: blob.size, ms: Math.round(performance.now() - t0) })
    } catch (e) {
      spikeLog({ test: 'S13', ok: false, error: String(e) })
    }
  }

  // ---- S18 pop-out
  let unlistenPopout: (() => void) | undefined
  async function popOut() {
    if (!remote || !viewRoom) throw new Error('watch a stream first')
    const who = streamer.trim() || firstStreamer(viewRoom)
    const opened = Date.now()
    unlistenPopout?.()
    unlistenPopout = await listen<{ firstFrameAt: number }>('spike-popout-first-frame', async (e) => {
      unlistenPopout?.()
      const lastMain = lastFrameAt
      for (const p of viewRoom?.remoteParticipants.values() ?? []) for (const tp of p.trackPublications.values()) if (tp.kind === 'video') tp.setSubscribed(false)
      spikeLog({ test: 'S18', event: 'handover', openedAt: opened, mainLastFrameAt: lastMain, popoutFirstFrameAt: e.payload.firstFrameAt, blipMs: e.payload.firstFrameAt - lastMain })
    })
    new WebviewWindow('popout', {
      url: `index.html?spike-popout=${encodeURIComponent(who ?? '')}&ch=${encodeURIComponent(need())}`,
      title: 'Pulse pop-out (spike)',
      width: 960,
      height: 540,
    })
    return { streamer: who }
  }

  // ---- S20 black/frozen watch on the local preview
  let blackOn = $state(false)
  let blackState = { since: 0, hash: 0, flagged: false }
  const blackCanvas = document.createElement('canvas')
  blackCanvas.width = 64
  blackCanvas.height = 36
  const previewEl = document.createElement('video')
  previewEl.muted = true
  async function blackSample() {
    const t = pub?.track?.mediaStreamTrack
    if (!t) return
    if (previewEl.srcObject === null || (previewEl.srcObject as MediaStream).getVideoTracks()[0] !== t) {
      previewEl.srcObject = new MediaStream([t])
      await previewEl.play().catch(() => {})
    }
    const g = blackCanvas.getContext('2d', { willReadFrequently: true })!
    g.drawImage(previewEl, 0, 0, 64, 36)
    const px = g.getImageData(0, 0, 64, 36).data
    let sum = 0
    let hash = 0
    for (let i = 0; i < px.length; i += 4) {
      const l = (px[i] + px[i + 1] + px[i + 2]) / 3
      sum += l
      hash = (hash * 31 + (l | 0)) | 0
    }
    const luma = sum / (64 * 36)
    const now = Date.now()
    const bad = luma < 8 || hash === blackState.hash
    if (!bad) blackState = { since: now, hash, flagged: false }
    else if (!blackState.flagged && now - blackState.since >= 3000) {
      blackState.flagged = true
      spikeLog({ test: 'S20', event: 'black_or_frozen', luma: Math.round(luma), black: luma < 8 })
    }
    blackState.hash = hash
  }

  // ---- timers
  const every2s = setInterval(() => {
    void sampleOut()
    void sampleIn()
    if (bgOn) {
      const fps = (rvfcFrames - bgPrevFrames) / 2
      bgPrevFrames = rvfcFrames
      spikeLog({ test: 'S6', visibility: document.visibilityState, focused: document.hasFocus(), rvfcFps: fps })
    }
  }, 2000)
  const every15s = setInterval(() => previewOn && void grabPreview(), 15_000)
  const every500ms = setInterval(() => blackOn && void blackSample(), 500)
  onDestroy(() => {
    for (const t of [every2s, every15s, every500ms, soakTimer]) clearInterval(t)
    unlistenPopout?.()
  })

  const run = (test: string, fn: () => Promise<object>) => () => void logResult(test, fn)
</script>

<aside class="spike">
  <h2>Spike panel <small>channel: {channel ?? 'not in voice'}</small></h2>

  <details open>
    <summary>Setup (S0)</summary>
    <p class="ua">{navigator.userAgent}</p>
    <input placeholder="GPU + driver version" bind:value={gpu} />
    <button onclick={run('S0', async () => ({ ua: navigator.userAgent, gpu }))}>Log</button>
  </details>

  <details>
    <summary>S1 / S5 Encode {pub ? '(live)' : ''}</summary>
    <select bind:value={opts.res}>{#each [720, 1080, 1440] as r (r)}<option value={r as Res}>{r}p</option>{/each}</select>
    <select bind:value={opts.fps}>{#each [15, 30, 60] as f (f)}<option value={f as Fps}>{f} fps</option>{/each}</select>
    <select bind:value={opts.codec}><option value="h264">H.264</option><option value="av1">AV1</option></select>
    <label><input type="checkbox" bind:checked={opts.lowLayer} /> low layer</label>
    <button onclick={run('S1', goLive)}>Go live</button>
    <button onclick={run('S1', stopLive)}>Stop</button>
    <div>
      <input placeholder="mark (S2)" bind:value={mark} />
      <button onclick={run('S2', async () => ({ mark }))}>Mark</button>
    </div>
  </details>

  <details>
    <summary>S4 / S7 Viewer {remote ? '(watching)' : ''}</summary>
    <input placeholder="streamer user id (blank: first live)" bind:value={streamer} />
    <label><input type="checkbox" bind:checked={rtxTime} /> rtx-time=300</label>
    <button onclick={run('S4', startWatch)}>Watch</button>
    <button onclick={run('S4', stopWatch)}>Stop</button>
    <label><input type="checkbox" bind:checked={dataSaver} onchange={run('S4', () => setDataSaver(dataSaver))} /> Data saver</label>
    <button onclick={run('S7', unlockStats)}>Unlock decoder stats (S7)</button>
    <!-- svelte-ignore a11y_media_has_caption -->
    <video bind:this={videoEl} autoplay playsinline muted></video>
  </details>

  <details>
    <summary>S6 Background</summary>
    <label><input type="checkbox" bind:checked={bgOn} /> log visibility / focus / fps every 2 s (stay in the background &gt; 2 min)</label>
  </details>

  <details>
    <summary>S3 Soak</summary>
    <button onclick={toggleSoak}>{soakOn ? 'Stop soak' : 'Start soak'}</button>
    <button onclick={run('S3', () => cycle('replace'))}>Cycle ×20 (replaceTrack)</button>
    <button onclick={run('S3', () => cycle('republish'))}>Cycle ×20 (republish)</button>
  </details>

  <details>
    <summary>S8 Picker</summary>
    <button onclick={run('S8', () => setHeight(560))}>Height 560</button>
    <button onclick={run('S8', () => setHeight(640))}>Height 640</button>
    <button onclick={run('S8', pickOnly)}>Pick</button>
    <input placeholder="window title" bind:value={pickTitle} />
    <button onclick={run('S8', findPid)}>Find PID by title</button>
  </details>

  <details>
    <summary>S9 / S19 Audio</summary>
    <select bind:value={audioMode}><option value="exclude">exclude Pulse</option><option value="include">include PID</option></select>
    <input placeholder="PID" bind:value={audioPid} size="7" />
    <select bind:value={tone}><option value="none">loopback</option><option value="left">left tone</option></select>
    <button onclick={run('S9', startAudio)}>Start stream audio</button>
    <button onclick={run('S9', stopAudio)}>Stop</button>
    <button onclick={run('S9', listenAudio)}>Listen</button>
    <button onclick={run('S9', measureAv)}>Measure A/V</button>
    {#if measuring}<button onclick={() => (measuring = false)}>Stop measuring</button>{/if}
  </details>

  <details>
    <summary>S12 Lifecycle</summary>
    <button onclick={killRenderer}>Kill renderer</button>
    <button onclick={run('S12', reconnectTest)}>Reconnect ×5</button>
  </details>

  <details>
    <summary>S13 Preview / S20 Capture check</summary>
    <label><input type="checkbox" bind:checked={previewOn} /> preview grab every 15 s (S13)</label>
    <label><input type="checkbox" bind:checked={blackOn} /> black/frozen watch (S20)</label>
  </details>

  <details>
    <summary>S18 Pop-out</summary>
    <button onclick={run('S18', popOut)}>Pop out</button>
  </details>
</aside>

<style>
  .spike {
    position: fixed;
    right: 8px;
    top: 40px;
    z-index: 9999;
    width: 380px;
    max-height: calc(100% - 56px);
    overflow: auto;
    padding: 8px;
    background: #101114ee;
    color: #ddd;
    border: 1px solid #f80;
    border-radius: 6px;
    font: 12px/1.4 monospace;
  }
  h2 { margin: 0 0 6px; font-size: 13px; color: #f80; }
  small { color: #999; font-weight: normal; }
  details { margin: 4px 0; }
  summary { cursor: pointer; }
  button, input, select { font: inherit; margin: 2px; }
  .ua { word-break: break-all; color: #999; }
  video { width: 100%; background: #000; margin-top: 4px; }
</style>
