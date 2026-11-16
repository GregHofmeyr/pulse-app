<script lang="ts">
  // A voice channel's YouTube watch party (#384), above the tiles. Everyone in the call sees that it's on and may join
  // it; joining opens the player (our embed page in party mode, YouTube's player driven from here), which follows the
  // party: `position + (server now - set_at)` while playing, seeked back when it drifts more than DRIFT_S. The video's
  // sound is each viewer's own (it never goes into the call). Leaving it closes the player and keeps you in voice.
  import { untrack } from 'svelte'
  import Icon from './Icon.svelte'
  import { YOUTUBE_FRAMING_ERROR } from '../lib/embeds'
  import { embed as origin, loadEmbedOrigin } from '../lib/linkPreviews.svelte'
  import { nameOfIn } from '../lib/selectors'
  import { serverNow } from '../lib/serverClock'
  import { app } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'
  import { clockText, correction, DRIFT_S, mayControl, partyPlayerSrc, partySeconds, watching, YT, type PlayerCommand, type PlayerInfo } from '../lib/watchParty'
  import { endWatchParty, joinWatchParty, leaveWatchParty, watchPartyAction } from '../lib/watchParty.svelte'
  import type { WatchParty } from '../lib/protocol/WatchParty'

  let { party, serverId, inCall }: {
    party: WatchParty
    serverId: string | null
    /** You're connected to this channel's call: only then may you watch. */
    inCall: boolean
  } = $props()

  loadEmbedOrigin()

  const me = $derived(app.state.me?.id ?? null)
  const nameOf = $derived(nameOfIn(() => app.state, serverId))
  const open = $derived(inCall && watching(party, me))
  const control = $derived(mayControl(party, me))
  const host = $derived(party.host_id === me)
  const title = $derived(party.title ?? 'A YouTube video')
  const others = $derived(party.viewers.filter((v) => v !== me).length)

  let error = $state('')
  let busy = $state(false)
  async function run(p: () => Promise<unknown>) {
    if (busy) return
    busy = true
    error = ''
    try {
      await p()
    } catch (e) {
      error = errorText(e)
    } finally {
      busy = false
    }
  }
  const act = (a: Parameters<typeof watchPartyAction>[1]) => run(() => watchPartyAction(party.channel_id, a))

  // The party's time, ticking while the player is open (the party is the truth; the player follows it).
  let now = $state(serverNow())
  $effect(() => {
    if (!open) return
    now = serverNow()
    const t = setInterval(() => (now = serverNow()), 500)
    return () => clearInterval(t)
  })
  const seconds = $derived(partySeconds(party, now))

  // ---- the player ----
  let frame = $state<HTMLIFrameElement>()
  let wrap = $state<HTMLElement>()
  let info = $state<PlayerInfo>({ state: null, time: null, duration: null })
  /** YouTube refused to play framed in the app (#88): it can still open in a window, unsynced. */
  let refused = $state(false)
  const VOLUME_KEY = 'pulse.watchVolume'
  let volume = $state(savedVolume())
  function savedVolume(): number {
    try {
      const v = Number(localStorage.getItem(VOLUME_KEY))
      return localStorage.getItem(VOLUME_KEY) !== null && Number.isFinite(v) ? Math.max(0, Math.min(100, v)) : 100
    } catch {
      return 100
    }
  }

  // One page per video, starting where the party is when it loads (the first sync seeks precisely).
  const src = $derived.by(() => {
    const id = party.video_id
    const o = origin.origin
    return o ? untrack(() => partyPlayerSrc(o, id, partySeconds(party, serverNow()))) : null
  })
  let endedFor: string | null = null
  let lastSeek = 0
  $effect(() => {
    void party.video_id
    untrack(() => {
      info = { state: null, time: null, duration: null }
      refused = false
      endedFor = null
      lastSeek = 0
    })
  })

  function send(c: PlayerCommand | { func: 'volume'; to: number }) {
    const o = origin.origin
    if (!o || !frame?.contentWindow) return
    frame.contentWindow.postMessage({ pulse: 'party-cmd', ...c }, new URL(o).origin)
  }

  /** Bring the player to the party. A seek waits a moment after the last one: the player's time catches up first. */
  function sync() {
    if (!open || refused) return
    const at = Date.now()
    for (const c of correction(party, info, serverNow())) {
      if (c.func === 'seek') {
        if (at - lastSeek < 2500) continue
        lastSeek = at
        info = { ...info, time: c.to }
      }
      send(c)
    }
  }
  $effect(() => {
    if (!open) return
    const t = setInterval(sync, 1000)
    return () => clearInterval(t)
  })
  // a change to the party applies at once (a seek by anyone goes straight through)
  $effect(() => {
    void party.position_ms
    void party.playing
    void party.set_at_ms
    untrack(() => {
      lastSeek = 0
      sync()
    })
  })

  // What the party page tells us: the player's state, time and duration, or an error. Only from our embed origin and
  // this panel's own frame.
  $effect(() => {
    if (!open || !origin.origin) return
    const from = new URL(origin.origin).origin
    let heard = false
    const onMessage = (e: MessageEvent) => {
      if (e.origin !== from || !frame || e.source !== frame.contentWindow) return
      const d: unknown = e.data
      if (!d || typeof d !== 'object' || (d as { pulse?: unknown }).pulse !== 'party') return
      const m = d as { state?: unknown; code?: unknown; player?: unknown; time?: unknown; duration?: unknown }
      if (m.state === 'error') {
        if (m.code === YOUTUBE_FRAMING_ERROR) refused = true
        else error = 'YouTube couldn’t play this video.'
        return
      }
      if (!heard) {
        heard = true
        send({ func: 'volume', to: volume })
      }
      const state = typeof m.player === 'number' ? m.player : info.state
      info = {
        state,
        time: typeof m.time === 'number' ? m.time : info.time,
        duration: typeof m.duration === 'number' && m.duration > 0 ? m.duration : info.duration,
      }
      // the video ran out: the host's player moves the party on (the queue's next, or it stops at the end)
      if (state === YT.ended && host && party.playing && endedFor !== party.video_id) {
        endedFor = party.video_id
        void watchPartyAction(party.channel_id, { action: 'next' }).catch(() => {})
      }
    }
    window.addEventListener('message', onMessage)
    return () => window.removeEventListener('message', onMessage)
  })

  function setVolume(v: number) {
    volume = v
    send({ func: 'volume', to: v })
    try {
      localStorage.setItem(VOLUME_KEY, String(v))
    } catch {
      /* ignore */
    }
  }

  // ---- controls ----
  const at = () => Math.round((info.time !== null && Math.abs(info.time - seconds) <= DRIFT_S * 2 ? info.time : seconds) * 1000)
  const togglePlay = () => act(party.playing ? { action: 'pause', position_ms: at() } : { action: 'play', position_ms: at() })
  let dragging = $state<number | null>(null)
  const duration = $derived(info.duration)
  function seekTo(s: number) {
    dragging = null
    void act({ action: 'seek', position_ms: Math.round(Math.max(0, s) * 1000) })
  }
  function fullscreen() {
    void wrap?.requestFullscreen?.().catch(() => {})
  }
  function openInWindow() {
    void api.openEmbedWindow('youtube', party.video_id, Math.floor(seconds) || null).catch((e) => (error = errorText(e)))
  }
</script>

<section class="party" class:open aria-label="Watch party">
  <div class="bar">
    <span class="ico" aria-hidden="true"><Icon name="film" size={16} /></span>
    <div class="what">
      <span class="title" data-tip={title}>{title}</span>
      <span class="sub">Watch party · {host ? 'you’re hosting' : `${nameOf(party.host_id)} is hosting`}{others ? ` · ${others} watching` : ''}</span>
    </div>
    {#if open}
      {#if host}
        <button type="button" class="btn btn-quiet sm everyone" aria-pressed={party.everyone_controls} data-tip="Let everyone watching play, pause, seek and change the video"
          onclick={() => act({ action: 'everyone_controls', on: !party.everyone_controls })}>{party.everyone_controls ? 'Everyone controls' : 'Let everyone control'}</button>
        <button type="button" class="btn btn-quiet sm" onclick={() => run(() => endWatchParty(party.channel_id))}>End</button>
      {/if}
      <button type="button" class="icon-btn icon-btn-sm" aria-label="Leave watch party" data-tip="Leave watch party (you stay in voice)" onclick={() => run(() => leaveWatchParty(party.channel_id))}><Icon name="x" size={14} /></button>
    {:else if inCall}
      <button type="button" class="btn btn-primary sm join" disabled={busy} onclick={() => run(() => joinWatchParty(party.channel_id))}>Join watch party</button>
    {:else}
      <span class="hint">Join voice to watch</span>
    {/if}
  </div>

  {#if error}<p class="error" role="alert">{error}</p>{/if}

  {#if open}
    {#if !origin.origin}
      <p class="note">Videos can’t play inside Pulse on this server.</p>
    {:else if refused}
      <div class="note">
        YouTube won’t play inside Pulse on this device.
        <button type="button" class="btn btn-secondary sm" onclick={openInWindow}>Open in a window</button>
        <small>It won’t stay in sync there.</small>
      </div>
    {:else}
      <div class="player" bind:this={wrap}>
        {#key src}
          <iframe bind:this={frame} {src} title="Watch party: {title}" sandbox="allow-scripts allow-same-origin allow-presentation"
            allow="autoplay; encrypted-media; fullscreen; picture-in-picture" referrerpolicy="strict-origin-when-cross-origin"></iframe>
        {/key}
      </div>
    {/if}
    <div class="controls">
      <button type="button" class="icon-btn" aria-label={party.playing ? 'Pause' : 'Play'} aria-disabled={!control}
        data-tip={control ? (party.playing ? 'Pause for everyone' : 'Play for everyone') : `${nameOf(party.host_id)} controls it`}
        onclick={() => control && togglePlay()}><Icon name={party.playing ? 'pause' : 'play'} size={18} /></button>
      <span class="time">{clockText(dragging ?? seconds)}{#if duration} / {clockText(duration)}{/if}</span>
      <input class="seek" type="range" min="0" max={duration ?? Math.max(1, Math.ceil(seconds))} step="1" value={dragging ?? Math.min(seconds, duration ?? seconds)}
        aria-label="Position" aria-valuetext={clockText(dragging ?? seconds)} disabled={!control || !duration}
        oninput={(e) => (dragging = +e.currentTarget.value)} onchange={(e) => seekTo(+e.currentTarget.value)} />
      {#if control && party.queue.length}
        <button type="button" class="btn btn-quiet sm" data-tip="Play the next video in the queue" onclick={() => act({ action: 'next' })}>Next</button>
      {/if}
      <span class="vol" data-tip="Your volume (only you hear it)"><Icon name="volume" size={16} />
        <input type="range" min="0" max="100" step="5" value={volume} aria-label="Video volume" oninput={(e) => setVolume(+e.currentTarget.value)} /></span>
      <button type="button" class="icon-btn" aria-label="Full screen" data-tip="Full screen" onclick={fullscreen}><Icon name="fullscreen" size={16} /></button>
    </div>
    {#if party.queue.length}
      <ol class="queue" aria-label="Up next">
        {#each party.queue.slice(0, 5) as item, i (i + item.video_id)}
          <li>
            <span class="q-title">{item.title ?? item.video_id}</span><span class="q-by">{nameOf(item.added_by)}</span>
            {#if control || item.added_by === me}
              <button type="button" class="icon-btn icon-btn-sm" aria-label="Remove from queue" data-tip="Remove from queue" onclick={() => act({ action: 'unqueue', index: i })}><Icon name="x" size={12} /></button>
            {/if}
          </li>
        {/each}
        {#if party.queue.length > 5}<li class="more">and {party.queue.length - 5} more</li>{/if}
      </ol>
    {/if}
  {/if}
</section>

<style>
  .party { margin: 12px 18px 0; padding: 10px 12px; border-radius: var(--radius-lg); background: var(--bg-2); border: 1px solid var(--line-soft); display: flex; flex-direction: column; gap: 8px; flex-shrink: 0; }
  .bar { display: flex; align-items: center; gap: 10px; min-width: 0; }
  .ico { width: 28px; height: 28px; border-radius: var(--radius-sm); display: grid; place-items: center; color: var(--accent-text); background: var(--accent-soft); flex-shrink: 0; }
  .what { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .title { font-weight: 600; font-size: 14px; color: var(--text); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .sub { font-size: 12px; color: var(--text-3); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .hint { font-size: 12px; color: var(--text-3); white-space: nowrap; }
  .sm { height: 28px; padding: 0 10px; font-size: 12px; white-space: nowrap; }
  .everyone[aria-pressed='true'] { color: var(--accent-text); background: var(--accent-soft); }
  .error { margin: 0; color: var(--danger-text); font-size: 13px; }
  .note { margin: 0; display: flex; flex-wrap: wrap; align-items: center; gap: 8px; font-size: 13px; color: var(--text-2); }
  .note small { color: var(--text-3); }
  /* 16:9, as wide as the panel allows without taking more than half the window's height */
  .player { width: min(100%, calc(50vh * 16 / 9)); aspect-ratio: 16 / 9; margin: 0 auto; border-radius: var(--radius-xs); overflow: hidden; background: var(--bg-inset); }
  .player iframe { display: block; width: 100%; height: 100%; border: 0; }
  .controls { display: flex; align-items: center; gap: 8px; min-width: 0; }
  .controls .icon-btn[aria-disabled='true'] { opacity: .5; cursor: not-allowed; }
  .controls .icon-btn:not([aria-disabled='true']):hover { color: var(--text); background: var(--bg-3); }
  .controls :global(svg) { flex-shrink: 0; }
  .time { font-size: 12px; color: var(--text-2); font-variant-numeric: tabular-nums; white-space: nowrap; }
  .seek { flex: 1; min-width: 60px; accent-color: var(--accent); }
  .vol { display: flex; align-items: center; gap: 4px; color: var(--text-3); }
  .vol input { width: 72px; accent-color: var(--accent); }
  input:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 2px; border-radius: 4px; }
  .queue { margin: 0; padding: 0; list-style: none; display: flex; flex-direction: column; gap: 2px; font-size: 12px; }
  .queue li { display: flex; align-items: center; gap: 8px; min-width: 0; color: var(--text-2); }
  .q-title { flex: 1; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .q-by, .more { color: var(--text-3); }
</style>
