<script lang="ts">
  // A badge's icon (#49): the emoji, or the badge's picture when it has one (#225), in a ring of its rarity's
  // colour, the tier as a numeral on a badge with tiers, a gold border, glow and ✦ for the first earner, "???" for
  // a hidden one, and the NEW tag until you've seen it. A picture is clipped round and fitted whole (a wide one
  // isn't cropped) on a neutral background.
  import { rarityVar, romanTier } from '../lib/badges.svelte'
  import { isMediaHash, mediaUrl } from '../lib/emoji'
  import type { BadgeRarity } from '../lib/protocol/BadgeRarity'

  let { icon, art = null, rarity, tier = 1, tiers = 1, first = false, hidden = false, size = 'case', isNew = false, label }: {
    icon: string
    /** The badge's picture, a media hash; the emoji shows without one. */
    art?: string | null
    rarity: BadgeRarity
    tier?: number
    /** How many tiers the badge has: the numeral shows only when there's more than one. */
    tiers?: number
    first?: boolean
    hidden?: boolean
    /** `case`: a trophy case tile; `show`: the smaller showcase on the card (and the unlock toast). */
    size?: 'case' | 'show'
    isNew?: boolean
    label?: string
  } = $props()

  const picture = $derived(!hidden && art && isMediaHash(art) ? art : null)
</script>

<!-- Without a label it's decoration (the text next to it names the badge), so it's hidden from screen readers. -->
<span class="badge-icon {rarity} {size}" class:first class:hidden class:has-art={picture} style:--ring-color={rarityVar(rarity)} role={label ? 'img' : undefined} aria-label={label} aria-hidden={label ? undefined : 'true'} data-tip={label}>
  {#if picture}
    <img class="art" src={mediaUrl(picture)} alt="" draggable="false" />
  {:else}
    <span class="glyph" aria-hidden="true">{hidden ? '???' : icon}</span>
  {/if}
  {#if tiers > 1 && !hidden}<span class="tier" aria-hidden="true">{romanTier(tier)}</span>{/if}
  {#if first}<span class="spark" aria-hidden="true">✦</span>{/if}
  {#if isNew}<span class="new">NEW</span>{/if}
</span>

<style>
  .badge-icon { position: relative; flex-shrink: 0; display: grid; place-items: center; width: 52px; height: 52px; border-radius: 50%; border: 3px solid var(--ring-color); background: var(--bg-2); }
  .badge-icon.show { width: 28px; height: 28px; border-width: 2px; }
  .glyph { font-size: 24px; line-height: 1; }
  .show .glyph { font-size: 14px; }
  .hidden .glyph { font-size: 13px; font-weight: 800; letter-spacing: .04em; color: var(--ring-color); }
  .show.hidden .glyph { font-size: 8px; }
  /* the picture fills the inside of the ring, clipped round, fitted whole on a neutral background */
  .art { display: block; width: 100%; height: 100%; border-radius: 50%; object-fit: contain; background: var(--bg-3); user-select: none; }
  /* the first earner: a gold outer ring and its glow around the rarity ring */
  .first { box-shadow: 0 0 0 2px var(--first-gold), 0 0 12px 2px var(--first-gold-soft); }
  .show.first { box-shadow: 0 0 0 1px var(--first-gold), 0 0 6px 1px var(--first-gold-soft); }
  .spark { position: absolute; top: -6px; right: -4px; font-size: 13px; line-height: 1; color: var(--first-gold); }
  .show .spark { top: -5px; right: -4px; font-size: 9px; }
  .tier { position: absolute; bottom: -6px; left: 50%; transform: translateX(-50%); min-width: 16px; padding: 1px 4px; border-radius: var(--radius-xs); background: var(--ring-color); color: var(--on-accent); font: 800 9px/1.2 ui-monospace, monospace; text-align: center; }
  .show .tier { bottom: -5px; padding: 0 2px; font-size: 7px; min-width: 10px; }
  .new { position: absolute; top: -7px; left: -8px; padding: 1px 4px; border-radius: 5px; background: var(--badge-new); color: var(--on-danger); font: 800 8px/1.3 ui-monospace, monospace; letter-spacing: .06em; }
</style>
