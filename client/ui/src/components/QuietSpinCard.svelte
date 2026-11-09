<script lang="ts">
  // A quiet picker wheel spin in the chat (#270): the same card a spin's line starts as, spinning to the server's
  // angle for everyone watching, but nothing is saved, so it isn't a message. Once it lands it shows the result for
  // a while (longer while pointed at or focused), then fades away (lib/wheels.svelte.ts quietSpins). Spin again and
  // Remove winner & spin work while it's shown, and are quiet too. A card that mounts later than a moment after it
  // arrived (the chat opened meanwhile) shows it landed. Landed, the winner shows in the result view over the wheel
  // (#271); otherwise quiet spins are as #270 left them.
  import { onDestroy, onMount, untrack } from 'svelte'
  import Name from './Name.svelte'
  import Wheel from './Wheel.svelte'
  import { exitFade } from '../lib/exit'
  import { errorText } from '../lib/tauri'
  import { LIVE_WINDOW_MS, WHEEL_LIMITS } from '../lib/wheel'
  import { quietNow, runSpin } from '../lib/wheelSpin'
  import { dropQuietSpin, holdQuietSpin, spinQuietly, type QuietCard } from '../lib/wheels.svelte'

  let { card, serverId }: { card: QuietCard; serverId: string | null } = $props()

  const spin = $derived(card.spin.spin)
  const n = $derived(spin.options.length)
  const winner = $derived(spin.options[spin.winner] ?? '')
  const label = $derived(spin.name ?? 'a quick wheel')

  // decided once, as it mounts
  const live = untrack(() => Date.now() - card.at < LIVE_WINDOW_MS)
  let landed = $state(untrack(() => !live))
  let rotation = $state(untrack(() => (live ? 0 : spin.angle)))

  onMount(() => {
    if (!live) return
    const { angle, options } = untrack(() => spin)
    return runSpin({ angle, count: options.length, quiet: quietNow(), onRotate: (deg) => (rotation = deg), onLanded: () => (landed = true) })
  })

  // pointed at or focused once it's landed: it stays until let go (then lingers a little)
  let hovered = $state(false)
  let held = false
  $effect(() => {
    const want = hovered && landed
    if (want === held) return
    held = want
    untrack(() => holdQuietSpin(card.spin.id, want))
  })
  // gone while held (the chat closed under the pointer): let go, so it still goes
  onDestroy(() => held && holdQuietSpin(card.spin.id, false))

  let busy = $state(false)
  let error = $state('')
  async function again(removeWinner: boolean) {
    if (busy) return
    busy = true
    error = ''
    const options = removeWinner ? spin.options.filter((_, i) => i !== spin.winner) : spin.options
    try {
      // a saved wheel's spin stays named; its options are this spin's (less the winner)
      await spinQuietly(card.spin.channel_id, spin.wheel_id ? { wheel_id: spin.wheel_id, options } : { options })
      dropQuietSpin(card.spin.id) // the new spin takes its place
    } catch (e) {
      error = errorText(e)
    } finally {
      busy = false
    }
  }
</script>

{#snippet who()}{#if spin.spinner_id}<Name userId={spin.spinner_id} {serverId} />{:else}Someone{/if}{/snippet}

<div class="wheel-card quiet-card" role="group" aria-label="Quiet spin" out:exitFade
  onmouseenter={() => (hovered = true)} onmouseleave={() => (hovered = false)}
  onfocusin={() => (hovered = true)} onfocusout={() => (hovered = false)}>
  <Wheel options={spin.options} {rotation} size={140} landed={landed ? spin.winner : null} result={landed ? winner : null} />
  <div class="body">
    <p class="caption" aria-live="polite">
      {#if !landed}{@render who()} is spinning {label}…
      {:else}{@render who()} spun {label} <span class="sr-only">→ {winner}</span>{/if}
    </p>
    <small class="note">Quiet spin · not saved</small>
    {#if landed}
      <span class="acts">
        <button type="button" class="link-btn" disabled={busy} onclick={() => again(false)}>Spin again</button>
        {#if n > WHEEL_LIMITS.minOptions}· <button type="button" class="link-btn" disabled={busy} onclick={() => again(true)}>Remove winner &amp; spin</button>{/if}
      </span>
    {/if}
    {#if error}<span class="err" role="alert">{error}</span>{/if}
  </div>
</div>

<style>
  /* the spin card (WheelLine's), indented like the system lines */
  .wheel-card { display: flex; align-items: center; gap: 16px; margin: 6px 18px 6px 30px; padding: 12px 16px; align-self: flex-start; background: var(--bg-2); border: 1px solid var(--line); border-radius: var(--radius-lg); }
  .body { display: flex; flex-direction: column; gap: 4px; min-width: 0; max-width: 280px; }
  .caption { margin: 0; font-size: 14px; color: var(--text-2); }
  .note { color: var(--text-3); font-size: 12px; }
  .acts { display: inline-flex; align-items: center; gap: 6px; flex-wrap: wrap; font-size: 13px; color: var(--text-3); }
  .acts .link-btn { font-size: 13px; }
  .acts .link-btn:disabled { opacity: .5; cursor: default; }
  .err { color: var(--danger-text); font-size: 12px; }
</style>
