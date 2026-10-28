<script lang="ts">
  // The trophy case panel's body: a grid of earned badges, or the empty state (plan reconciliation 15, 17).
  import Icon from './Icon.svelte'
  import { badgeInfo } from '../lib/badges'
  import { formatMemberSince } from '../lib/profileFormat'
  import type { Badge } from '../lib/protocol/Badge'

  let { badges }: { badges: Badge[] } = $props()
</script>

{#if badges.length === 0}
  <p class="empty">No trophies yet</p>
{:else}
  <ul class="grid">
    {#each badges as b (b.key)}
      {@const info = badgeInfo(b.key)}
      <li>
        <span class="ico"><Icon name={info.icon} size={22} /></span>
        <span class="nm">{info.name}</span>
        <span class="when">{formatMemberSince(b.earned_at)}</span>
      </li>
    {/each}
  </ul>
{/if}

<style>
  .empty { margin: 0; color: var(--card-text-2); }
  .grid { list-style: none; margin: 0; padding: 0; display: grid; grid-template-columns: repeat(auto-fill, minmax(110px, 1fr)); gap: 10px; }
  li { display: flex; flex-direction: column; align-items: center; gap: 4px; text-align: center; padding: 10px 6px; border-radius: 10px; background: var(--panel-bottom); }
  .ico { color: var(--card-text); }
  .nm { font-weight: 600; color: var(--card-text); }
  .when { font-size: 12px; color: var(--card-text-2); }
</style>
