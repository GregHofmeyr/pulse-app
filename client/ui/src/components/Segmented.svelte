<script lang="ts" generics="T">
  // A row of mutually exclusive options (a radio group) for settings panes.
  let { label, options, value, onchange }: {
    label: string
    options: { value: T; label: string }[]
    value: T
    onchange: (v: T) => void
  } = $props()
</script>

<div class="seg" role="radiogroup" aria-label={label}>
  {#each options as o}
    <button role="radio" aria-checked={value === o.value} class:on={value === o.value} onclick={() => onchange(o.value)}>{o.label}</button>
  {/each}
</div>

<style>
  .seg { display: inline-flex; align-self: flex-start; border: 1px solid var(--bg-4); border-radius: 10px; overflow: hidden; }
  .seg button { padding: 8px 14px; border: 0; background: transparent; color: var(--text-2); font-weight: 600; }
  .seg button.on { background: var(--accent); color: var(--on-accent); }
</style>
