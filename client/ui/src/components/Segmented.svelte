<script lang="ts" generics="T">
  // A row of mutually exclusive options (a radio group) for settings panes and popovers. Too many options for its
  // box (the inbox's six tabs, #454): the row scrolls sideways instead of spilling out, the wheel scrolls it too,
  // a soft fade marks the side with more, and the picked option is always scrolled into view.
  // With `tabs` it is a tablist instead (Settings → Profile's five tabs, #716): roving tab order, arrows / Home / End
  // move and pick, and each tab controls its panel.
  let { label, options, value, onchange, size = 'md', fill = false, tabs }: {
    label: string
    options: { value: T; label: string }[]
    value: T
    onchange: (v: T) => void
    /** `sm`: tighter padding and 13px text, for a popover's tab row. */
    size?: 'md' | 'sm'
    /** Stretch across its box with every option the same width (the emoji/GIF picker's tabs, #461). */
    fill?: boolean
    /** A tablist: tab `v` is `#${tabs}-tab-${v}` and controls `#${tabs}-panel-${v}`. */
    tabs?: string
  } = $props()

  let el: HTMLDivElement | undefined = $state()
  let more = $state({ start: false, end: false })

  function edges() {
    if (!el) return
    more = { start: el.scrollLeft > 1, end: el.scrollLeft + el.clientWidth < el.scrollWidth - 1 }
  }

  // Keep the picked option in view: only the row's own scroll moves (scrollIntoView would scroll a settings pane).
  $effect(() => {
    void value
    const on = el?.querySelector<HTMLElement>('.on')
    if (el && on) {
      const l = on.offsetLeft
      const r = l + on.offsetWidth
      if (l < el.scrollLeft) el.scrollLeft = Math.max(0, l - 24)
      else if (r > el.scrollLeft + el.clientWidth) el.scrollLeft = r - el.clientWidth + 24
    }
    edges()
  })

  function tabKeys(e: KeyboardEvent) {
    const i = options.findIndex((o) => o.value === value)
    const n = options.length
    const to = e.key === 'ArrowRight' ? (i + 1) % n : e.key === 'ArrowLeft' ? (i + n - 1) % n : e.key === 'Home' ? 0 : e.key === 'End' ? n - 1 : -1
    if (to < 0) return
    e.preventDefault()
    onchange(options[to].value)
    el?.querySelectorAll<HTMLElement>('[role=tab]')[to]?.focus()
  }

  $effect(() => {
    if (!el) return
    const node = el
    // A vertical wheel scrolls the row sideways while it has more to show (a mouse has no sideways wheel).
    const wheel = (e: WheelEvent) => {
      if (node.scrollWidth <= node.clientWidth || Math.abs(e.deltaY) <= Math.abs(e.deltaX)) return
      e.preventDefault()
      node.scrollLeft += e.deltaY
    }
    node.addEventListener('wheel', wheel, { passive: false })
    const ro = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(edges)
    ro?.observe(node)
    return () => {
      node.removeEventListener('wheel', wheel)
      ro?.disconnect()
    }
  })
</script>

{#if tabs}
  <div bind:this={el} class="seg" class:sm={size === 'sm'} class:fill class:more-start={more.start} class:more-end={more.end}
    role="tablist" aria-label={label} tabindex="-1" onscroll={edges} onkeydown={tabKeys}>
    {#each options as o}
      <button type="button" role="tab" id="{tabs}-tab-{o.value}" aria-selected={value === o.value} aria-controls="{tabs}-panel-{o.value}"
        tabindex={value === o.value ? 0 : -1} class:on={value === o.value} onclick={() => onchange(o.value)}>{o.label}</button>
    {/each}
  </div>
{:else}
  <div bind:this={el} class="seg" class:sm={size === 'sm'} class:fill class:more-start={more.start} class:more-end={more.end}
    role="radiogroup" aria-label={label} onscroll={edges}>
    {#each options as o}
      <button type="button" role="radio" aria-checked={value === o.value} class:on={value === o.value} onclick={() => onchange(o.value)}>{o.label}</button>
    {/each}
  </div>
{/if}

<style>
  /* max-width + overflow-x: never wider than its box; it scrolls instead, and the fade says there's more. The mask
     only reads alpha: --bg-1 is just an opaque colour token here (every theme's is solid). */
  .seg { --fade-start: 0px; --fade-end: 0px; position: relative; display: inline-flex; align-self: flex-start; max-width: 100%; min-width: 0; border: 1px solid var(--bg-4); border-radius: var(--radius-md); overflow-x: auto; overflow-y: hidden; overscroll-behavior-x: contain;
    mask-image: linear-gradient(to right, transparent 0, var(--bg-1) var(--fade-start), var(--bg-1) calc(100% - var(--fade-end)), transparent 100%); }
  .seg.more-start { --fade-start: 28px; }
  .seg.more-end { --fade-end: 28px; }
  .seg button { flex: none; padding: 8px 14px; border: 0; background: transparent; color: var(--text-2); font-weight: 600; white-space: nowrap; }
  .seg.fill { display: flex; align-self: stretch; }
  .seg.fill button { flex: 1 1 0; }
  .seg.sm button { padding: 6px 10px; font-size: 13px; }
  .seg button:not(.on):hover { background: var(--bg-3); color: var(--text); }
  .seg button.on { background: var(--accent); color: var(--on-accent); }
</style>
