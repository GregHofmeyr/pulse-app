<script lang="ts">
  // Unlock toasts (#49): "Badge unlocked: <name>" for the earner only, bottom-left so they never cover
  // the stream toasts; each fades by itself (lib/badges.svelte.ts), a click opens your trophy case.
  import Icon from './Icon.svelte'
  import BadgeIcon from './BadgeIcon.svelte'
  import { badgeStore, openOwnCase, rarityLabel, rarityVar } from '../lib/badges.svelte'
</script>

<div class="stack" aria-live="polite">
  {#each badgeStore.toasts as t (t.id)}
    <div class="toast fade-in" role="status" style:--ring-color={rarityVar(t.rarity)}>
      <button type="button" class="open" onclick={() => { badgeStore.dismiss(t.id); openOwnCase() }}>
        <BadgeIcon icon={t.icon} rarity={t.rarity} size="show" />
        <span class="text">
          <span><strong>Badge unlocked:</strong> {t.name}</span>
          <small>{rarityLabel(t.rarity)}{t.tier > 1 ? ` · tier ${t.tier}` : ''}</small>
        </span>
      </button>
      <button type="button" class="x" aria-label="Dismiss" onclick={() => badgeStore.dismiss(t.id)}><Icon name="x" size={14} /></button>
    </div>
  {/each}
</div>

<style>
  .stack { position: fixed; left: 18px; bottom: 18px; z-index: 40; display: flex; flex-direction: column; gap: 8px; align-items: flex-start; }
  .toast { display: flex; align-items: center; gap: 8px; padding: 8px 10px 8px 12px; background: var(--surface-menu); border: 1px solid var(--line-menu); border-left: 3px solid var(--ring-color); border-radius: 12px; box-shadow: var(--shadow-float); font-size: 13px; }
  .open { display: flex; align-items: center; gap: 10px; padding: 0; border: 0; background: none; color: var(--text); font: inherit; text-align: left; cursor: pointer; }
  .text { display: flex; flex-direction: column; gap: 1px; }
  small { font-size: 11px; font-weight: 700; letter-spacing: .04em; text-transform: uppercase; color: var(--ring-color); }
  .x { width: 24px; height: 24px; flex-shrink: 0; padding: 0; border: 0; border-radius: 6px; background: transparent; color: var(--text-3); display: grid; place-items: center; }
  .x:hover { background: var(--bg-4); color: var(--text); }
</style>
