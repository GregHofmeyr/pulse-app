<script lang="ts">
  // The Quote of the Year banner (spec §5.2): over every channel of a server while its vote is live.
  // "Vote" opens the voting window.
  import PixelSpill from './PixelSpill.svelte'
  import QotyVoting from './QotyVoting.svelte'
  import { leftLabel, liveQoty } from '../lib/quotes'
  import { app, clock } from '../lib/store.svelte'

  let { serverId }: { serverId: string } = $props()

  const now = $derived(Date.parse(clock.nowIso))
  const e = $derived(liveQoty(app.state, serverId, now))
  let voting = $state(false)
</script>

{#if e}
  <div class="qoty-banner" role="region" aria-label="Quote of the Year {e.year}">
    <PixelSpill seed={e.id} cols={45} rows={8} />
    <span class="mark" aria-hidden="true">“”</span>
    <span class="live">LIVE</span>
    <div class="txt">
      <b>Quote of the Year {e.year}</b>
      <small>{leftLabel(e.ends_at, now)} · {e.voters} of {e.members} have voted</small>
    </div>
    <button type="button" class="vote" aria-haspopup="dialog" onclick={() => (voting = true)}>Vote</button>
  </div>
  {#if voting}<QotyVoting {serverId} onClose={() => (voting = false)} />{/if}
{/if}

<style>
  .qoty-banner { position: relative; overflow: hidden; flex-shrink: 0; display: flex; align-items: center; gap: 12px; min-height: 56px; padding: 8px 14px; background: #0e1013; border-bottom: 1px solid color-mix(in srgb, var(--quote-green) 35%, transparent); }
  .mark, .live, .txt, .vote { position: relative; } /* over the spill */
  .qoty-banner :global(.pixel-spill) { position: absolute; left: 0; top: 0; height: 100%; width: 315px; }
  .mark { font-family: Georgia, serif; font-size: 30px; line-height: .6; padding-top: 10px; font-weight: 700; color: var(--on-quote-green); }
  .live { font: 700 10px/1 ui-monospace, monospace; letter-spacing: .12em; color: var(--on-quote-green); background: rgba(255, 255, 255, .35); border-radius: 4px; padding: 3px 5px; }
  .txt { margin-left: 110px; min-width: 0; display: flex; flex-direction: column; text-shadow: 0 0 6px #000, 0 1px 0 #000; }
  .txt b { font-size: 14.5px; }
  .txt small { color: var(--text-2); font-size: 12px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .vote { margin-left: auto; height: 32px; padding: 0 16px; border: 0; border-radius: 8px; background: var(--quote-green); color: var(--on-quote-green); font-size: 13px; font-weight: 700; box-shadow: 0 2px 0 var(--quote-green-deep); white-space: nowrap; }
</style>
