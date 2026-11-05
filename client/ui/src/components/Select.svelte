<script lang="ts">
  // Themed dropdown (native <select> popups take the GTK theme's colours on Linux).
  import Icon from './Icon.svelte'

  type Option = { value: string | null; label: string }
  let { value = $bindable(), options, label, onchange, drop = 'down' }: { value: string | null; options: Option[]; label: string; onchange?: () => void; /** Which way the list opens: `up` for a control at the bottom of a clipped box. */ drop?: 'down' | 'up' } = $props()

  let open = $state(false)
  let active = $state(0)
  const current = $derived(options.find((o) => o.value === value) ?? options[0])

  function pick(o: Option) {
    value = o.value
    open = false
    onchange?.()
  }
  function key(e: KeyboardEvent) {
    if (!open && (e.key === 'Enter' || e.key === ' ' || e.key === 'ArrowDown')) {
      e.preventDefault()
      open = true
      active = Math.max(0, options.findIndex((o) => o.value === value))
    } else if (open) {
      if (e.key === 'Escape') open = false
      else if (e.key === 'ArrowDown') active = Math.min(options.length - 1, active + 1)
      else if (e.key === 'ArrowUp') active = Math.max(0, active - 1)
      else if (e.key === 'Enter') pick(options[active])
      else return
      e.preventDefault()
    }
  }
</script>

<div class="select" data-drop={drop}>
  <button type="button" class="trigger" aria-haspopup="listbox" aria-expanded={open} aria-label={label}
    onclick={() => (open = !open)} onkeydown={key} onblur={() => setTimeout(() => (open = false), 120)}>
    <span>{current?.label}</span><Icon name="chevron" size={16} />
  </button>
  {#if open}
    <ul class="list menu-surface pop-in" role="listbox" aria-label={label}>
      {#each options as o, i}
        <li role="option" aria-selected={o.value === value} class:active={i === active}
          onmousedown={(e) => { e.preventDefault(); pick(o) }} onmouseenter={() => (active = i)}>
          {o.label}
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .select { position: relative; }
  /* the text fields' look (.input: 36px on bg-1), so a dropdown in Settings sits level with the fields beside it (#173) */
  .trigger { width: 100%; height: 36px; padding: 0 12px; border: 1px solid var(--bg-4); border-radius: var(--radius-md); background: var(--bg-1); color: var(--text); display: flex; align-items: center; justify-content: space-between; font-size: 14px; font-weight: 400; letter-spacing: 0; text-align: left; }
  .trigger:hover { border-color: var(--line-menu); }
  /* its options keep the trigger's 14px */
  .list { position: absolute; z-index: 5; top: calc(100% + 4px); left: 0; right: 0; margin: 0; padding: 4px; list-style: none; max-height: 240px; overflow-y: auto; --pop-origin: top; }
  .select[data-drop='up'] .list { top: auto; bottom: calc(100% + 4px); --pop-origin: bottom; }
  li { padding: 9px 10px; border-radius: var(--radius-sm); font-size: 14px; font-weight: 400; letter-spacing: 0; color: var(--text-2); cursor: pointer; }
  li.active { background: var(--bg-4); color: var(--text); }
  li[aria-selected='true'] { color: var(--text); font-weight: 600; }
</style>
