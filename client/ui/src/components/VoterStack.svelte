<script lang="ts">
  import Avatar from './Avatar.svelte'

  let { userIds, nameOf, max = 5 }: {
    userIds: string[]
    nameOf: (userId: string | null) => string
    /** Avatars shown before the rest collapse into "+N". */
    max?: number
  } = $props()

  const shown = $derived(userIds.slice(0, max))
  const extra = $derived(userIds.length - shown.length)
  // every name, however many (the avatars stop at `max`, the tip does not)
  const tip = $derived.by(() => {
    const names = userIds.map((id) => nameOf(id))
    return names.length < 2 ? (names[0] ?? '') : `${names.slice(0, -1).join(', ')} and ${names[names.length - 1]}`
  })
</script>

{#if userIds.length > 0}
  <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
  <span class="stack" data-tip={tip} role="img" aria-label={tip} tabindex="0">
    {#each shown as id (id)}<Avatar {id} name={nameOf(id)} size={18} />{/each}
    {#if extra > 0}<span class="more">+{extra}</span>{/if}
  </span>
{/if}

<style>
  .stack { display: inline-flex; align-items: center; }
  .stack :global(.av) { box-shadow: 0 0 0 2px var(--bg-1); }
  .stack :global(.av + .av) { margin-left: -6px; }
  .more { margin-left: 5px; font-size: 11px; color: var(--text-3); }
</style>
