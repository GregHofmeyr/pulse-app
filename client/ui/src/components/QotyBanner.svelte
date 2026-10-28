<script lang="ts">
  // The Quote of the Year banner (spec §5.2): over every channel of a server while its vote is live.
  // "Vote" opens the voting window.
  import PixelSpill from './PixelSpill.svelte'
  import QotyVoting from './QotyVoting.svelte'
  import { leftLabel, liveQoty } from '../lib/quotes'
  import { qotyWindow } from '../lib/qotyWindow.svelte'
  import { app, clock } from '../lib/store.svelte'

  let { serverId }: { serverId: string } = $props()

  const now = $derived(Date.parse(clock.nowIso))
  const e = $derived(liveQoty(app.state, serverId, now))
  const voting = qotyWindow(() => serverId, () => e)
</script>

{#if e}
  <div class="qoty-banner" role="region" aria-label="Quote of the Year {e.year}">
    <PixelSpill seed={e.id} cols={45} rows={8} />
    <span class="qoty-mark mark" aria-hidden="true">“”</span>
    <span class="qoty-live live">LIVE</span>
    <div class="txt">
      <b>Quote of the Year {e.year}</b>
      <small>{leftLabel(e.ends_at, now)} · {e.voters} of {e.members} have voted</small>
    </div>
    <button type="button" class="qoty-btn vote" aria-haspopup="dialog" onclick={voting.show}>Vote</button>
  </div>
  {#if voting.open}<QotyVoting {serverId} onClose={voting.close} />{/if}
{/if}

<style>
  .qoty-banner { position: relative; overflow: hidden; flex-shrink: 0; display: flex; align-items: center; gap: 12px; min-height: 56px; padding: 8px 14px; background: var(--quote-surface); border-bottom: 1px solid color-mix(in srgb, var(--quote-green) 35%, transparent); }
  .mark, .live, .txt, .vote { position: relative; } /* over the spill */
  .qoty-banner :global(.pixel-spill) { position: absolute; left: 0; top: 0; height: 100%; width: 315px; }
  .mark { font-size: 30px; padding-top: 10px; color: var(--on-quote-green); }
  .live { background: color-mix(in srgb, var(--text) 35%, transparent); }
  .txt { margin-left: 110px; min-width: 0; display: flex; flex-direction: column; text-shadow: 0 0 6px var(--quote-surface), 0 1px 0 var(--quote-surface); }
  .txt b { font-size: 14.5px; }
  .txt small { color: var(--text-2); font-size: 12px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .vote { margin-left: auto; padding: 0 16px; }
</style>
