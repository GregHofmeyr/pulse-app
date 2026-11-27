<script lang="ts">
  // A card in miniature (#727): a banner, an avatar and two lines of text, in the given theme's own colours and its own
  // text colours (the card's, not the app theme's), so a tile shows what the theme looks like. Used by the Looks
  // tiles and the Card theme popover.
  import { cardVars } from '../../lib/cardTheme'

  let { top, bottom }: { top: string; bottom: string } = $props()
  const style = $derived(Object.entries(cardVars(top, bottom)).map(([k, v]) => `${k}: ${v}`).join('; '))
</script>

<span class="mc" {style} aria-hidden="true">
  <span class="mc-banner"></span><span class="mc-avatar"></span><span class="mc-line"></span><span class="mc-line short"></span>
</span>

<style>
  /* fills its tile (the tile sets the size); the card's own gradient and edge */
  .mc { position: relative; display: block; width: 100%; height: 100%; min-width: 0; min-height: 0; overflow: hidden; border-radius: inherit; background: linear-gradient(to bottom, var(--card-top), var(--card-bottom)); box-shadow: inset 0 0 0 1px var(--card-edge); }
  .mc-banner { position: absolute; top: 0; left: 0; right: 0; height: 34%; background: linear-gradient(to bottom, var(--card-border-top), var(--card-top)); }
  .mc-avatar { position: absolute; left: 10%; top: 22%; width: 22px; height: 22px; border-radius: 50%; background: var(--card-text); box-shadow: 0 0 0 3px var(--card-top); }
  .mc-line { position: absolute; left: 10%; right: 10%; top: 66%; height: 5px; border-radius: 3px; background: var(--card-text-2); opacity: .75; }
  .mc-line.short { top: 80%; right: 40%; opacity: .45; }
</style>
