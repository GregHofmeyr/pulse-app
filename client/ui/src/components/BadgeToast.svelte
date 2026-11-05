<script lang="ts">
  // Unlock toasts (#49): "Badge unlocked: <name>" for the earner only, top-left under the header (#139) so
  // they never cover the stream toasts; each fades by itself unless hovered or focused (lib/badges.svelte.ts),
  // a click opens your trophy case. A fresh one bursts a little confetti from behind it (~1 s, CSS only),
  // unless motion is off (lib/animate.svelte.ts: reduced motion, live, or "Animate emoji" on never).
  import { untrack } from 'svelte'
  import Icon from './Icon.svelte'
  import { exitFade } from '../lib/exit'
  import BadgeIcon from './BadgeIcon.svelte'
  import { badgeStore, openOwnCase, rarityLabel, rarityVar } from '../lib/badges.svelte'
  import { motion, shouldCelebrate, watchMotion } from '../lib/animate.svelte'
  import { windowFocus } from '../lib/windowFocus.svelte'

  watchMotion()

  // Decided once, when a toast first shows: turning motion back on later never replays a burst. An
  // unfocused window gets none either (like the toast fades in app.css: nobody would see it).
  const celebrated = new Map<number, boolean>()
  function celebrate(id: number): boolean {
    return untrack(() => {
      if (!celebrated.has(id)) celebrated.set(id, windowFocus.focused && shouldCelebrate(motion))
      return celebrated.get(id)!
    })
  }
  // forget toasts that have gone
  $effect(() => {
    const live = new Set(badgeStore.toasts.map((t) => t.id))
    for (const id of celebrated.keys()) if (!live.has(id)) celebrated.delete(id)
  })

  // The pieces: spread round an ellipse (the toast is wide and short), each with its own colour, spin and delay.
  // The stack hugs the window's left edge, so leftward pieces travel less and the burst isn't cut off there.
  const COLORS = ['var(--ring-color)', 'var(--accent)', 'var(--warn)', 'var(--ok)', 'var(--danger)']
  const PIECES = Array.from({ length: 16 }, (_, i) => {
    const a = (i / 16) * Math.PI * 2 + (i % 2 ? 0.2 : -0.1)
    const reach = 1 + (i % 3) * 0.25
    const cos = Math.cos(a)
    return {
      dx: `${Math.round(cos * (cos < 0 ? 90 : 150) * reach)}px`,
      dy: `${Math.round(Math.sin(a) * 48 * reach)}px`,
      spin: `${(i % 2 ? 1 : -1) * (240 + (i % 4) * 90)}deg`,
      delay: `${(i % 4) * 30}ms`,
      color: COLORS[i % COLORS.length],
    }
  })
</script>

<div class="stack" aria-live="polite">
  {#each badgeStore.toasts as t (t.id)}
    <!-- the item is the stacking context (no background of its own), so the confetti paints under the toast -->
    <div class="item" style:--ring-color={rarityVar(t.rarity)} out:exitFade>
    {#if celebrate(t.id)}
      <span class="confetti" aria-hidden="true">
        {#each PIECES as p, i (i)}<i style:--dx={p.dx} style:--dy={p.dy} style:--spin={p.spin} style:--c={p.color} style:animation-delay={p.delay}></i>{/each}
      </span>
    {/if}
    <!-- hovering or focusing a toast holds it (its fade waits until both have left) -->
    <div
      class="toast fade-in"
      role="status"
      onmouseenter={() => badgeStore.hold(t.id, 'hover')}
      onmouseleave={() => badgeStore.release(t.id, 'hover')}
      onfocusin={() => badgeStore.hold(t.id, 'focus')}
      onfocusout={() => badgeStore.release(t.id, 'focus')}
    >
      <button type="button" class="open" onclick={() => { badgeStore.dismiss(t.id); openOwnCase() }}>
        <!-- no label: the text beside it says it -->
        <BadgeIcon icon={t.icon} rarity={t.rarity} size="show" />
        <span class="text">
          <span><strong>Badge unlocked:</strong> {t.name}</span>
          <small>{rarityLabel(t.rarity)}{t.tier > 1 ? ` · tier ${t.tier}` : ''}</small>
        </span>
      </button>
      <button type="button" class="icon-btn icon-btn-xs" aria-label="Dismiss" onclick={() => badgeStore.dismiss(t.id)}><Icon name="x" size={14} /></button>
    </div>
    </div>
  {/each}
</div>

<style>
  .stack { position: fixed; left: 18px; top: 62px; z-index: 40; display: flex; flex-direction: column; gap: 8px; align-items: flex-start; }
  .item { position: relative; isolation: isolate; }
  /* the shared .toast (app.css), with the badge's rarity as its left edge */
  .toast { position: relative; border-left: 3px solid var(--ring-color); }
  .open { display: flex; align-items: center; gap: 10px; padding: 0; border: 0; background: none; color: var(--text); font: inherit; text-align: left; cursor: pointer; }
  .text { display: flex; flex-direction: column; gap: 1px; }
  small { font-size: 11px; font-weight: 700; letter-spacing: .04em; text-transform: uppercase; color: var(--ring-color); }
  /* behind the toast (z -1 in the item's stacking context; the toast is positioned at z auto above it): the
     pieces start hidden under it and fly out past its edges */
  .confetti { position: absolute; inset: 0; z-index: -1; pointer-events: none; }
  .confetti i { position: absolute; left: 50%; top: 50%; width: 6px; height: 9px; margin: -4px 0 0 -3px; border-radius: 1px; background: var(--c); opacity: 0; animation: confetti-burst .9s cubic-bezier(.2, .7, .3, 1) forwards; }
  @keyframes confetti-burst {
    0% { opacity: 1; transform: translate(0, 0) rotate(0); }
    70% { opacity: 1; }
    100% { opacity: 0; transform: translate(var(--dx), calc(var(--dy) + 18px)) rotate(var(--spin)); }
  }
  @media (prefers-reduced-motion: reduce) { .confetti { display: none; } }
</style>
