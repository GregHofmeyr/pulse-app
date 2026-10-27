<script lang="ts">
  // The slim Quote of the Year strip under a voice channel's tiles while the server's vote is live
  // (spec §5.2): who leads and the time left, and "Open voting".
  import PixelSpill from './PixelSpill.svelte'
  import QotyVoting from './QotyVoting.svelte'
  import { leaders, leftLabel, liveQoty, speakersOf } from '../lib/quotes'
  import { app, clock } from '../lib/store.svelte'

  let { serverId }: { serverId: string } = $props()

  const now = $derived(Date.parse(clock.nowIso))
  const e = $derived(liveQoty(app.state, serverId, now))
  // A tie names every leader, each person once: "sam & kim lead with 2".
  const lead = $derived.by(() => {
    const top = e ? leaders(e) : []
    if (!top.length) return 'no votes yet'
    const names = speakersOf(app.state, serverId, top.map((o) => o.message))
    return `${names} ${top.length > 1 ? 'lead' : 'leads'} with ${top[0].votes}`
  })
  let voting = $state(false)
</script>

{#if e}
  <div class="qoty-strip" role="region" aria-label="Quote of the Year {e.year}">
    <PixelSpill seed={e.id} cols={8} rows={4} />
    <span class="qoty-mark mark" aria-hidden="true">“”</span>
    <span class="txt"><b>Quote of the Year</b> <span class="lead">· {lead} · {leftLabel(e.ends_at, now)}</span></span>
    <button type="button" class="qoty-btn open" aria-haspopup="dialog" onclick={() => (voting = true)}>Open voting</button>
  </div>
  {#if voting}<QotyVoting {serverId} onClose={() => (voting = false)} />{/if}
{/if}

<style>
  .qoty-strip { position: relative; overflow: hidden; flex-shrink: 0; margin: 0 20px; display: flex; align-items: center; gap: 10px; padding: 7px 10px; border-radius: 10px; background: var(--bg-1); border: 1px solid color-mix(in srgb, var(--quote-green) 30%, transparent); font-size: 13px; }
  .mark, .txt, .open { position: relative; } /* over the spill */
  .qoty-strip :global(.pixel-spill) { position: absolute; left: 0; top: 0; height: 100%; width: 72px; }
  .mark { font-size: 20px; padding-top: 6px; color: var(--on-quote-green); }
  .txt { margin-left: 34px; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .lead { color: var(--text-2); }
  .open { margin-left: auto; height: 28px; padding: 0 10px; font-size: 12px; }
</style>
