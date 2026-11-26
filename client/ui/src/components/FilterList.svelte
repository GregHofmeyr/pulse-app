<script lang="ts">
  // One section of a filter panel (#663): a header, the "all" row, then one row per choice with its count; the picked
  // one is pressed. Builds' and Clips' Games, and Clips' People, are each one of these.

  let { heading, kind, allLabel, allCount, rows, selected, onSelect }: {
    heading: string
    /** Names the list (`.games`, `.people`): a class for styling hooks and tests. */
    kind: string
    allLabel: string
    /** The "all" row's count; nothing shows while unknown (never a guess). */
    allCount: number | null | undefined
    /** A null count shows no number (not loaded yet). */
    rows: { id: string; name: string; count: number | null }[]
    selected: string | undefined
    onSelect: (id: string | undefined) => void
  } = $props()

  const uid = $props.id()
</script>

<section>
  <h3 class="menu-label" id="{uid}-h">{heading}</h3>
  <div class="list {kind}" role="group" aria-labelledby="{uid}-h">
    <button type="button" aria-pressed={!selected} onclick={() => onSelect(undefined)}>
      <span class="name">{allLabel}</span><span class="n">{allCount ?? ''}</span>
    </button>
    {#each rows as r (r.id)}
      <button type="button" aria-pressed={selected === r.id} onclick={() => onSelect(r.id)}>
        <span class="name">{r.name}</span><span class="n">{r.count ?? ''}</span>
      </button>
    {/each}
  </div>
</section>

<style>
  /* text lands --inset (set by the panel) in from the content box, like every row of the panel */
  h3 { margin: 0; padding: 0 var(--inset); }
  .list { display: flex; flex-direction: column; gap: 1px; margin-top: -2px; }
  .list button { display: flex; align-items: center; gap: 8px; min-width: 0; padding: 6px var(--inset); border: 0; border-radius: var(--radius-sm); background: transparent; color: var(--text-2); font: inherit; font-size: 14px; text-align: left; }
  .list button:hover { background: var(--bg-3); color: var(--text); }
  .list button[aria-pressed='true'] { background: var(--bg-4); color: var(--text); font-weight: 600; }
  .name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .n { flex-shrink: 0; color: var(--text-3); font-size: 12px; font-variant-numeric: tabular-nums; }
</style>
