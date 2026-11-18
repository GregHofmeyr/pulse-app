<script lang="ts">
  // A voice channel's YouTube watch party (#384), above the tiles. Everyone in the call sees that it's on and may join
  // it; joining opens the player (our embed page in party mode, YouTube's player driven from here), which follows the
  // party: `position + (server now - set_at)` while playing, seeked back when it drifts more than DRIFT_S. The video's
  // sound is each viewer's own (it never goes into the call). Leaving it closes the player and keeps you in voice.
  import { untrack } from 'svelte'
  import Icon from './Icon.svelte'
  import WatchQueue from './WatchQueue.svelte'
  import { YOUTUBE_FRAMING_ERROR } from '../lib/embeds'
  import { embed as origin, loadEmbedOrigin } from '../lib/linkPreviews.svelte'
  import { nameOfIn } from '../lib/selectors'
  import { serverNow } from '../lib/serverClock'
  import { app } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'
  import { isMediaHash, mediaUrl } from '../lib/emoji'
  import { CLIP_S, clockText, correction, DRIFT_S, embedOf, mayControl, maySkip, partyPlayerSrc, partySeconds, sourceLabel, untitled, upNextIn, watching, YT, type PlayerCommand, type PlayerInfo } from '../lib/watchParty'
  import { endWatchParty, enqueueLink, joinWatchParty, leaveWatchParty, watchPartyAction } from '../lib/watchParty.svelte'
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
  // what it plays (#459): YouTube (synced), a Twitch VOD (synced), a live channel (play and pause, until skipped) or
  // a clip (started together, no control)
  const source = $derived(party.source ?? 'youtube')
  const twitch = $derived(source !== 'youtube')
  const live = $derived(source === 'twitch_channel')
  const clip = $derived(source === 'twitch_clip')
  const provider = $derived(twitch ? 'Twitch' : 'YouTube')
  const title = $derived(party.title ?? untitled(source))
  const others = $derived(party.viewers.filter((v) => v !== me).length)
  const skip = $derived(maySkip(party, me))

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

  // One page per video, starting where the party is when it loads (the first sync seeks precisely). Only a new video
  // (or embed origin) makes a new page: every party change is a new `party` object, and a page built from its position
  // would reload the player on each pause, play and seek, and the page autoplays, so a pause never stuck.
  const videoId = $derived(party.video_id)
  const src = $derived.by(() => {
    const m = { source, id: videoId }
    const o = origin.origin
    return o ? untrack(() => partyPlayerSrc(o, m, partySeconds(party, serverNow()))) : null
  })
  let endedFor: string | null = null
  let lastSeek = 0
  // a new video starts the player's bookkeeping afresh (only a new video: not every change to the party)
  $effect(() => {
    void videoId
    void source
    untrack(() => {
      info = { state: null, time: null, duration: null }
      refused = false
      silent = false
      endedFor = null
      lastSeek = 0
    })
  })
  /** A Twitch VOD's or channel's player that never said anything back (Twitch refusing the app as its parent shows
   *  only inside its frame, #88): it can still open in a window, unsynced. */
  let silent = $state(false)
  $effect(() => {
    if (!open || !twitch || clip || !src) return
    void src
    const t = setTimeout(() => {
      if (info.state === null) silent = true
    }, 15_000)
    return () => clearTimeout(t)
  })
  // a clip says nothing back: the party moves on from it a clip's longest after it started (whoever may skip it)
  $effect(() => {
    if (!open || !clip || !skip || !party.playing || seconds < CLIP_S || endedFor === party.video_id) return
    endedFor = party.video_id
    void watchPartyAction(party.channel_id, { action: 'next', from: party.video_id }).catch(() => {})
  })

  /** When we last told the player to pause: its own pause (below) is one we didn't ask for. */
  let pauseSentAt = 0
  function send(c: PlayerCommand | { func: 'volume'; to: number }) {
    const o = origin.origin
    if (!o || !frame?.contentWindow) return
    if (c.func === 'pause') pauseSentAt = Date.now()
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
        else error = `${provider} couldn’t play this.`
        return
      }
      silent = false
      if (!heard) {
        heard = true
        send({ func: 'volume', to: volume })
      }
      const state = typeof m.player === 'number' ? m.player : info.state
      const was = info.state
      info = {
        state,
        time: typeof m.time === 'number' ? m.time : info.time,
        duration: typeof m.duration === 'number' && m.duration > 0 ? m.duration : info.duration,
      }
      // the player paused itself while the party plays (a media key, the system's media controls): from someone who
      // may control it, that's a pause for everyone, not something to play straight over
      if (was === YT.playing && state === YT.paused && party.playing && control && Date.now() - pauseSentAt > 2000) {
        void act({ action: 'pause', position_ms: Math.round((info.time ?? seconds) * 1000) })
      }
      // the video ran out: the player of anyone who may skip it moves the party on for everyone (the queue's next, or
      // it stops at the end); `from` makes several players saying so at once move it on once (#464)
      if (state === YT.ended && skip && party.playing && endedFor !== party.video_id) {
        endedFor = party.video_id
        void watchPartyAction(party.channel_id, { action: 'next', from: party.video_id }).catch(() => {})
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
  // ---- the queue (#464) ----
  let queueOpen = $state(false)
  let pill = $state<HTMLButtonElement>()
  const upNext = $derived(open && !refused ? upNextIn(party, seconds, duration) : null)
  // a live channel plays until skipped: what's next shows the whole time
  const liveNext = $derived(open && live && party.queue.length > 0)
  const nextItem = $derived(party.queue[0])
  const nextThumb = $derived(nextItem?.thumb && isMediaHash(nextItem.thumb.hash) ? mediaUrl(nextItem.thumb.hash) : null)
  const skipNow = () => act({ action: 'next', from: party.video_id })
  // a link dropped anywhere on the party goes in the queue (anyone in the call may add)
  let dropping = $state(false)
  const linkDrag = (e: DragEvent) => !!e.dataTransfer && [...e.dataTransfer.types].some((t) => t === 'text/uri-list' || t === 'text/plain')
  function dragOver(e: DragEvent) {
    if (!inCall || !linkDrag(e)) return
    e.preventDefault()
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'copy'
    dropping = true
  }
  function dropLink(e: DragEvent) {
    dropping = false
    if (!inCall || !e.dataTransfer) return
    const text = e.dataTransfer.getData('text/uri-list') || e.dataTransfer.getData('text/plain')
    if (!text) return
    e.preventDefault()
    void run(() => enqueueLink(party.channel_id, text))
  }

  function openInWindow() {
    const e = embedOf({ source, id: party.video_id })
    const start = source === 'youtube' ? Math.floor(seconds) || null : null
    void api.openEmbedWindow(e.provider, e.id, start).catch((err) => (error = errorText(err)))
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<section class="party" class:open class:dropping aria-label="Watch party" ondragover={dragOver} ondragleave={() => (dropping = false)} ondrop={dropLink}>
  <div class="bar">
    <span class="ico" aria-hidden="true"><Icon name="film" size={16} /></span>
    <div class="what">
      <span class="title" data-tip={title}>{title}</span>
      <span class="sub">Watch party · {host ? 'you’re hosting' : `${nameOf(party.host_id)} is hosting`}{others ? ` · ${others} watching` : ''}</span>
    </div>
    {#if inCall}
      <button bind:this={pill} type="button" class="btn btn-quiet sm pill" aria-expanded={queueOpen} aria-haspopup="dialog" data-tip="What plays next. Anyone in the call can add to it."
        onclick={() => (queueOpen = !queueOpen)}><Icon name="list" size={14} />Queue{party.queue.length ? ` · ${party.queue.length}` : ''}</button>
      {#if queueOpen}<WatchQueue {party} {me} {nameOf} opener={pill} onClose={() => (queueOpen = false)} />{/if}
    {/if}
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
        {provider} won’t play inside Pulse on this device.
        <button type="button" class="btn btn-secondary sm" onclick={openInWindow}>Open in a window</button>
        <small>It won’t stay in sync there.</small>
      </div>
    {:else}
      <div class="player" bind:this={wrap}>
        {#key src}
          <iframe bind:this={frame} {src} title="Watch party: {title}" sandbox="allow-scripts allow-same-origin allow-presentation"
            allow="autoplay; encrypted-media; fullscreen; picture-in-picture" referrerpolicy="strict-origin-when-cross-origin"></iframe>
        {/key}
        {#if !clip}
          <!-- clicks on the video are ours, not the player's: its pausing itself on a click only got played over. A clip
               keeps its own controls (it has no control API; its pause is each viewer's own) -->
          <button type="button" class="shield" tabindex="-1" aria-label={control ? (party.playing ? 'Pause for everyone' : 'Play for everyone') : 'Video'}
            class:control onclick={() => control && togglePlay()} ondblclick={fullscreen}></button>
        {/if}
      </div>
      {#if silent}
        <div class="note">
          Twitch isn’t answering inside Pulse on this device, so it may not stay in sync.
          <button type="button" class="btn btn-secondary sm" onclick={openInWindow}>Open in a window</button>
        </div>
      {/if}
    {/if}
    <div class="controls">
      {#if clip}
        <span class="badge" data-tip="Twitch clips can’t be controlled from Pulse">Clip</span>
        <span class="time kind">Started together · pausing is just for you</span>
      {:else}
        <button type="button" class="icon-btn" aria-label={party.playing ? 'Pause' : 'Play'} aria-disabled={!control}
          data-tip={control ? (party.playing ? 'Pause for everyone' : 'Play for everyone') : `${nameOf(party.host_id)} controls it`}
          onclick={() => control && togglePlay()}><Icon name={party.playing ? 'pause' : 'play'} size={18} /></button>
        {#if live}
          <span class="badge live">Live</span>
          <span class="time kind">Plays until skipped</span>
        {:else}
          <span class="time">{clockText(dragging ?? seconds)}{#if duration} / {clockText(duration)}{/if}</span>
          <input class="seek" type="range" min="0" max={duration ?? Math.max(1, Math.ceil(seconds))} step="1" value={dragging ?? Math.min(seconds, duration ?? seconds)}
            aria-label="Position" aria-valuetext={clockText(dragging ?? seconds)} disabled={!control || !duration}
            oninput={(e) => (dragging = +e.currentTarget.value)} onchange={(e) => seekTo(+e.currentTarget.value)} />
        {/if}
      {/if}
      {#if skip && party.queue.length}
        <button type="button" class="btn btn-quiet sm" data-tip="Skip to the next video in the queue, for everyone" onclick={skipNow}>Skip</button>
      {/if}
      {#if !clip}
        <span class="vol" data-tip="Your volume (only you hear it)"><Icon name="volume" size={16} />
          <input type="range" min="0" max="100" step="5" value={volume} aria-label="Video volume" oninput={(e) => setVolume(+e.currentTarget.value)} /></span>
      {/if}
      <button type="button" class="icon-btn" aria-label="Full screen" data-tip="Full screen" onclick={fullscreen}><Icon name="fullscreen" size={16} /></button>
    </div>
    {#if (upNext !== null || liveNext) && nextItem}
      <div class="up-next" role="status">
        <span class="un-thumb" aria-hidden="true">{#if nextThumb}<img src={nextThumb} alt="" />{:else}<Icon name="film" size={16} />{/if}</span>
        <span class="un-text">
          <span class="un-label">{upNext !== null ? `Up next · in ${upNext}s` : 'Up next · when this is skipped'}</span>
          <span class="un-title"><span class="badge" class:live={nextItem.source === 'twitch_channel'}>{sourceLabel(nextItem.source)}</span> {nextItem.title ?? untitled(nextItem.source)}</span>
          <span class="un-by">added by {nextItem.added_by === me ? 'you' : nameOf(nextItem.added_by)}</span>
        </span>
        {#if skip}<button type="button" class="btn btn-secondary sm" onclick={skipNow}>Skip</button>{/if}
      </div>
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
  .player { position: relative; width: min(100%, calc(50vh * 16 / 9)); aspect-ratio: 16 / 9; margin: 0 auto; border-radius: var(--radius-xs); overflow: hidden; background: var(--bg-inset); }
  .player iframe { display: block; width: 100%; height: 100%; border: 0; }
  .shield { position: absolute; inset: 0; padding: 0; border: 0; background: transparent; cursor: default; }
  .shield.control { cursor: pointer; }
  .controls { display: flex; align-items: center; gap: 8px; min-width: 0; }
  .controls .icon-btn[aria-disabled='true'] { opacity: .5; cursor: not-allowed; }
  .controls .icon-btn:not([aria-disabled='true']):hover { color: var(--text); background: var(--bg-3); }
  .controls :global(svg) { flex-shrink: 0; }
  .time { font-size: 12px; color: var(--text-2); font-variant-numeric: tabular-nums; white-space: nowrap; }
  .seek { flex: 1; min-width: 60px; accent-color: var(--accent); }
  .vol { display: flex; align-items: center; gap: 4px; color: var(--text-3); }
  .vol input { width: 72px; accent-color: var(--accent); }
  input:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 2px; border-radius: 4px; }
  .bar { position: relative; }
  .pill { display: inline-flex; align-items: center; gap: 6px; }
  .pill[aria-expanded='true'] { color: var(--accent-text); background: var(--accent-soft); }
  .party.dropping { border-color: var(--focus-ring); }
  /* "Up next" (#464): near the end, before it moves on */
  .badge { display: inline-block; padding: 0 5px; border-radius: var(--radius-xs); font-size: 10px; font-weight: 700; line-height: 16px; text-transform: uppercase; letter-spacing: .03em; color: var(--text-2); background: var(--bg-inset); vertical-align: 1px; }
  .badge.live { color: var(--danger-text); }
  .kind { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; }
  .up-next { display: flex; align-items: center; gap: 10px; padding: 8px; border-radius: var(--radius-sm); background: var(--bg-3); min-width: 0; }
  .un-thumb { width: 64px; height: 36px; border-radius: var(--radius-xs); overflow: hidden; background: var(--bg-inset); display: grid; place-items: center; color: var(--text-3); flex-shrink: 0; }
  .un-thumb img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .un-text { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .un-label { font-size: 11px; font-weight: 600; color: var(--accent-text); font-variant-numeric: tabular-nums; }
  .un-title { font-size: 13px; color: var(--text); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .un-by { font-size: 11px; color: var(--text-3); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
</style>
