<script lang="ts">
  // Themed dropdown (native <select> popups take the GTK or Windows theme's colours, #455): every dropdown in Pulse.
  // A button that opens a listbox. Keys as a native select: ↑ ↓ Home End move, Enter or Space picks, Esc and Tab
  // close, and typing jumps to the option starting with what was typed. Focus stays on the button, which points at
  // the highlighted option (aria-activedescendant; the select-only combobox pattern). The list is moved out to the page (`portal`, so a scrolling pane
  // or a card can't clip it) and placed by the button, kept inside the window: it opens upward when there's no room
  // below, and scrolls when it's longer than the room it has.
  import { tick } from 'svelte'
  import Icon from './Icon.svelte'
  import { portal } from '../lib/actions'
  import { placePopover } from '../lib/menuPlace'
  import { TYPE_AHEAD_MS, typeAheadIndex } from '../lib/typeAhead'

  type Option = { value: string | null; label: string; disabled?: boolean }
  let { value = $bindable(), options, label, onchange, drop = 'down', disabled = false, size = 'md' }: {
    value: string | null
    options: Option[]
    label: string
    /** Called after a pick changed `value`, with the new value. */
    onchange?: (value: string | null) => void
    /** Which way the list prefers to open: `up` for a control at the bottom of a box. It still flips to the other
     *  side when the window has no room on this one. */
    drop?: 'down' | 'up'
    disabled?: boolean
    /** `sm`: the 32px `.input-sm` size, for a settings row. */
    size?: 'md' | 'sm'
  } = $props()

  const uid = $props.id()
  const listId = `${uid}-list`
  const optId = (i: number) => `${uid}-opt-${i}`
  /** The tallest the list grows before it scrolls. */
  const MAX_H = 240

  let open = $state(false)
  let active = $state(0)
  let place = $state('')
  let trigger: HTMLButtonElement | undefined = $state()
  let list: HTMLUListElement | undefined = $state()
  const current = $derived(options.find((o) => o.value === value) ?? options[0])

  const enabled = (i: number) => !!options[i] && !options[i].disabled
  /** The next enabled option from `from` in direction `dir`, or `from` when there's none. */
  function step(from: number, dir: 1 | -1) {
    for (let i = from + dir; i >= 0 && i < options.length; i += dir) if (enabled(i)) return i
    return from
  }

  function position() {
    if (!trigger) return
    const r = trigger.getBoundingClientRect()
    const height = Math.min(list?.scrollHeight || options.length * 36 + 10, MAX_H)
    const width = Math.min(Math.max(r.width, 160), window.innerWidth - 16)
    const p = placePopover(r, { width, height }, undefined, 'start', 4, drop === 'up' ? 'above' : 'below')
    const y = p.below ? `top: ${p.top}px` : `bottom: ${p.bottom}px`
    place = `left: ${p.left}px; ${y}; width: ${width}px; max-height: ${Math.min(MAX_H, p.maxHeight)}px; --pop-origin: ${p.below ? 'top' : 'bottom'}`
  }

  function scrollActive() {
    const el = open ? document.getElementById(optId(active)) : null
    el?.scrollIntoView?.({ block: 'nearest' })
  }

  async function show(at?: number) {
    if (disabled || !options.length) return
    const sel = options.findIndex((o) => o.value === value)
    active = at ?? (enabled(sel) ? sel : step(-1, 1))
    open = true
    position()
    await tick()
    position() // again with the list's real height
    scrollActive()
  }
  function hide() {
    open = false
    typed = ''
  }

  function pick(i: number) {
    if (!enabled(i)) return
    const o = options[i]
    hide()
    if (o.value === value) return
    value = o.value
    onchange?.(o.value)
  }

  let typed = ''
  let typedAt = 0
  function typeAhead(ch: string) {
    const now = Date.now()
    typed = now - typedAt > TYPE_AHEAD_MS ? ch : typed + ch
    typedAt = now
    const i = typeAheadIndex(options.map((o) => o.label), typed, open ? active : Math.max(0, options.findIndex((o) => o.value === value)), (k) => !enabled(k))
    if (i < 0) return
    if (open) {
      active = i
      void tick().then(scrollActive)
    } else void show(i)
  }

  function key(e: KeyboardEvent) {
    if (disabled) return
    const typing = e.key.length === 1 && e.key !== ' ' && !e.ctrlKey && !e.metaKey && !e.altKey
    if (typing || (e.key === ' ' && typed && Date.now() - typedAt <= TYPE_AHEAD_MS)) {
      e.preventDefault()
      typeAhead(e.key)
      return
    }
    if (!open) {
      if (e.key === 'Enter' || e.key === ' ' || e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault()
        void show()
      }
      return
    }
    if (e.key === 'Tab') return hide()
    if (e.key === 'Escape') hide() // preventDefault below: a modal around it doesn't close too
    else if (e.key === 'ArrowDown') active = step(active, 1)
    else if (e.key === 'ArrowUp') active = step(active, -1)
    else if (e.key === 'Home') active = step(-1, 1)
    else if (e.key === 'End') active = step(options.length, -1)
    else if (e.key === 'Enter' || e.key === ' ') pick(active)
    else return
    e.preventDefault()
    e.stopPropagation()
    void tick().then(scrollActive)
  }

  // While it's open: a scroll anywhere but the list (it would drift from its button) or a resize closes it.
  $effect(() => {
    if (!open) return
    const scrolled = (e: Event) => {
      if (list && e.target instanceof Node && list.contains(e.target)) return
      hide()
    }
    document.addEventListener('scroll', scrolled, true)
    window.addEventListener('resize', hide)
    return () => {
      document.removeEventListener('scroll', scrolled, true)
      window.removeEventListener('resize', hide)
    }
  })
  $effect(() => {
    if (disabled && open) hide()
  })
</script>

<div class="select" class:sm={size === 'sm'} data-drop={drop}>
  <button bind:this={trigger} type="button" class="trigger" role="combobox" aria-haspopup="listbox" aria-expanded={open} aria-controls={open ? listId : undefined}
    aria-activedescendant={open ? optId(active) : undefined} aria-label={label} data-value={value ?? ''} {disabled}
    onclick={() => (open ? hide() : void show())} onkeydown={key} onblur={() => setTimeout(hide, 120)}>
    <span class="current">{current?.label}</span><Icon name="chevron" size={16} />
  </button>
  {#if open}
    <!-- a press on the list keeps the focus on the button (so its blur doesn't close the list first) -->
    <ul bind:this={list} id={listId} class="list menu-surface pop-in" class:sm={size === 'sm'} role="listbox" aria-label={label} tabindex="-1"
      style={place} use:portal onmousedown={(e) => e.preventDefault()}>
      {#each options as o, i}
        <li id={optId(i)} role="option" aria-selected={o.value === value} aria-disabled={o.disabled ? 'true' : undefined} data-value={o.value ?? ''}
          class:active={i === active} onmousedown={(e) => { e.preventDefault(); pick(i) }} onmouseenter={() => { if (enabled(i)) active = i }}>
          {o.label}
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .select { position: relative; min-width: 0; }
  /* the text fields' look (.input: 36px on bg-1), so a dropdown in Settings sits level with the fields beside it (#173) */
  .trigger { width: 100%; height: 36px; padding: 0 12px; border: 1px solid var(--bg-4); border-radius: var(--radius-md); background: var(--bg-1); color: var(--text); display: flex; align-items: center; justify-content: space-between; gap: 8px; font-size: 14px; font-weight: 400; letter-spacing: 0; text-align: left; }
  .trigger:hover:not(:disabled) { border-color: var(--line-menu); }
  .trigger:disabled { opacity: .6; cursor: default; }
  /* a long option ends in an ellipsis on the button; the list shows it whole */
  .current { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .trigger > :global(svg) { flex-shrink: 0; color: var(--text-3); }
  /* .input-sm's size */
  .select.sm .trigger { height: 32px; padding: 0 10px; border-radius: var(--radius-sm); font-size: 13px; }
  /* fixed by the button (placed in script), on top of menus; its options keep the button's text size */
  .list { position: fixed; z-index: 70; margin: 0; padding: 4px; list-style: none; max-width: calc(100vw - 16px); overflow-y: auto; outline: none; }
  li { padding: 9px 10px; border-radius: var(--radius-sm); font-size: 14px; font-weight: 400; letter-spacing: 0; color: var(--text-2); cursor: pointer; overflow-wrap: anywhere; }
  .list.sm li { padding: 7px 10px; font-size: 13px; }
  li.active { background: var(--bg-4); color: var(--text); }
  li[aria-selected='true'] { color: var(--text); font-weight: 600; }
  li[aria-disabled='true'] { opacity: .45; cursor: default; }
</style>
