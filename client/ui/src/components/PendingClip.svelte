<script lang="ts">
  // A clip you're sending, in its channel's feed until it posts: the local poster, caption and game, and how far
  // it's got. Only your own client shows it (the server knows nothing of it until it posts).
  import { untrack } from 'svelte'
  import { convertFileSrc } from '@tauri-apps/api/core'
  import Icon from './Icon.svelte'
  import { formatDuration } from '../lib/clips'
  import { app } from '../lib/store.svelte'
  import { api, errorText, type ClipJob } from '../lib/tauri'

  let { job }: { job: ClipJob } = $props()

  const serverId = $derived(app.state.channels[job.channel_id]?.server_id ?? '')
  const game = $derived(job.game_id ? (app.state.games[serverId]?.find((g) => g.id === job.game_id)?.name ?? null) : null)
  const thumb = $derived(job.thumb_path ? convertFileSrc(job.thumb_path) : null)
  const percent = $derived(Math.round(Math.min(1, Math.max(0, job.progress)) * 100))
  const status = $derived(
    job.stage === 'preparing' ? `Preparing… ${percent}%`
    : job.stage === 'uploading' ? `Uploading… ${percent}%`
    : job.stage === 'posting' || job.stage === 'done' ? 'Posting…' // done: until the clip takes its place
    : '',
  )

  // One press each: a second Cancel while it posts would race the first (the core waits for the confirm).
  let cancelling = $state(false)
  let retrying = $state(false)
  let problem = $state('')
  // A refused Retry or Cancel is about that press: once the job moves on, it goes.
  let lastStage = untrack(() => job.stage)
  $effect.pre(() => {
    if (job.stage === lastStage) return
    lastStage = job.stage
    problem = ''
  })

  async function cancel() {
    if (cancelling) return
    cancelling = true
    problem = ''
    try {
      await api.cancelClip(job.id)
    } catch (e) {
      problem = errorText(e)
      cancelling = false
    }
  }
  async function retry() {
    if (retrying) return
    retrying = true
    problem = ''
    try {
      await api.retryClip(job.id) // its progress arrives as clip://job
    } catch (e) {
      problem = errorText(e)
    } finally {
      retrying = false
    }
  }
</script>

<article class="pending-clip" data-job-id={job.id} aria-busy={job.stage !== 'failed' && job.stage !== 'done'}>
  <div class="media">
    {#if thumb}<img src={thumb} alt="" decoding="async" />{:else}<span class="ph" aria-hidden="true"><Icon name="film" size={26} /></span>{/if}
    <span class="duration">{formatDuration(job.kept_ms)}</span>
  </div>
  <div class="info">
    {#if job.caption}<p class="caption">{job.caption}</p>{/if}
    {#if game}<span class="game">{game}</span>{/if}
    {#if job.stage === 'failed'}
      <p class="failed" role="alert">
        Upload failed — <button type="button" class="link" disabled={retrying || cancelling} onclick={() => void retry()}>Retry</button>
      </p>
      {#if job.error}<p class="why">{job.error}</p>{/if}
    {:else if status}
      <p class="status" role="status">{status}</p>
      {#if job.stage === 'preparing' || job.stage === 'uploading'}<div class="bar" aria-hidden="true"><span style:width="{percent}%"></span></div>{/if}
    {/if}
    {#if problem}<p class="why">{problem}</p>{/if}
    {#if job.stage !== 'done'}
      <div class="actions"><button type="button" class="cancel" disabled={cancelling} onclick={() => void cancel()}>Cancel</button></div>
    {/if}
  </div>
</article>

<style>
  .pending-clip { min-width: 0; display: flex; flex-direction: column; border-radius: 12px; background: var(--bg-2); border: 1px dashed var(--bg-3); }
  .media { position: relative; aspect-ratio: 16 / 9; background: #000; border-radius: 12px 12px 0 0; overflow: hidden; }
  .media img { width: 100%; height: 100%; object-fit: cover; display: block; opacity: .75; }
  .ph { position: absolute; inset: 0; display: grid; place-items: center; color: var(--text-3); }
  .duration { position: absolute; right: 8px; bottom: 8px; padding: 1px 6px; border-radius: 6px; background: rgba(0, 0, 0, .75); color: #fff; font-size: 11px; font-variant-numeric: tabular-nums; }
  .info { padding: 8px 10px 10px; display: flex; flex-direction: column; align-items: flex-start; gap: 5px; min-width: 0; }
  .caption { margin: 0; font-size: 13px; overflow-wrap: anywhere; white-space: pre-wrap; }
  .game { padding: 1px 8px; border-radius: 10px; background: var(--bg-3); color: var(--text-2); font-size: 11px; font-weight: 600; white-space: nowrap; }
  .status { margin: 0; font-size: 12px; color: var(--text-2); font-variant-numeric: tabular-nums; }
  .bar { align-self: stretch; height: 4px; border-radius: 2px; background: var(--bg-3); overflow: hidden; }
  .bar span { display: block; height: 100%; background: var(--accent); transition: width .2s; }
  .failed { margin: 0; font-size: 12px; color: var(--danger); }
  .why { margin: 0; font-size: 12px; color: var(--text-3); overflow-wrap: anywhere; }
  .link { padding: 0; border: 0; background: none; color: var(--accent); font: inherit; text-decoration: underline; }
  .actions { align-self: stretch; display: flex; justify-content: flex-end; }
  .cancel { padding: 4px 10px; border: 0; border-radius: 8px; background: var(--bg-3); color: var(--text-2); font-size: 12px; }
  .cancel:hover:not(:disabled) { color: var(--text); }
  .cancel:disabled, .link:disabled { opacity: .5; }
</style>
