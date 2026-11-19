<script lang="ts">
  // Settings › Privacy › Blocked people (#474): everyone you've blocked, in the order you blocked them, with how each
  // shows. "+ Add person" opens the people picker (each is blocked with a line, the default; change it on their row).
  // A row's right-click menu (or Shift+F10 / the menu key on it): Show a line / Hide completely / Unblock. Their own
  // person menus everywhere else have Block… and Unblock too.
  import { onMount, tick } from 'svelte'
  import Avatar from './Avatar.svelte'
  import Icon from './Icon.svelte'
  import PeoplePicker from './PeoplePicker.svelte'
  import { app } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'
  import { displayName } from '../lib/selectors'
  import { clickOutside } from '../lib/actions'
  import { placeMenu } from '../lib/menuPlace'
  import { isMenuKey, menuPoint } from '../lib/participantMenu'
  import type { BlockMode } from '../lib/protocol/BlockMode'

  let error = $state('')
  const blocked = $derived(app.state.blocks.map((b) => ({ id: b.user_id, mode: b.mode, name: displayName(app.state, null, b.user_id) })))
  const candidates = $derived.by(() => {
    const s = app.state
    const ids = new Set([...Object.values(s.members).flatMap((ms) => ms.map((m) => m.user.id)), ...Object.values(s.dmMembers).flat()])
    ids.delete(s.me?.id ?? '')
    for (const b of s.blocks) ids.delete(b.user_id)
    return [...ids]
  })
  let picking = $state(false)

  async function run(work: () => Promise<unknown>) {
    error = ''
    try {
      await work()
    } catch (e) {
      error = errorText(e)
    }
  }
  function add(ids: string[]) {
    picking = false
    // each BlocksChanged that follows puts them on the list
    void run(async () => {
      for (const id of ids) await api.setBlock(id, 'line')
    })
  }

  // a row's menu
  let menu = $state<{ id: string; name: string; mode: BlockMode; x: number; y: number; opener: HTMLElement } | null>(null)
  let menuEl = $state<HTMLElement>()
  let place = $state({ left: 0, top: 0, origin: 'top left' })
  function open(e: MouseEvent | KeyboardEvent, p: { id: string; name: string; mode: BlockMode }) {
    e.preventDefault()
    const opener = e.currentTarget as HTMLElement
    const at = menuPoint(e, opener)
    menu = { ...p, ...at, opener }
    place = { left: at.x, top: at.y, origin: 'top left' }
    void tick().then(() => {
      if (!menuEl || !menu) return
      place = placeMenu(menu.x, menu.y, menuEl.offsetWidth, menuEl.offsetHeight)
      menuEl.querySelector<HTMLElement>('[role^=menuitem]')?.focus()
    })
  }
  function close(refocus: boolean) {
    const m = menu
    menu = null
    if (refocus && m?.opener.isConnected) m.opener.focus()
  }
  /** Set (or with null, lift) the block of the person the menu is for, then close it. */
  function pick(mode: BlockMode | null) {
    const id = menu?.id
    close(true)
    if (id) void run(() => api.setBlock(id, mode))
  }
  function menuKey(e: KeyboardEvent) {
    const items = menuEl ? [...menuEl.querySelectorAll<HTMLElement>('[role^=menuitem]')] : []
    const at = items.indexOf(document.activeElement as HTMLElement)
    const go = (i: number) => items[(i + items.length) % items.length]?.focus()
    if (e.key === 'ArrowDown') { e.preventDefault(); go(at + 1) }
    else if (e.key === 'ArrowUp') { e.preventDefault(); go(at - 1) }
    else if (e.key === 'Home') { e.preventDefault(); go(0) }
    else if (e.key === 'End') { e.preventDefault(); go(-1) }
    else if (e.key === 'Escape' || e.key === 'Tab') { e.preventDefault(); e.stopPropagation(); close(true) }
  }
  onMount(() => () => (menu = null))
</script>

<div class="settings-field">
  <h3 class="settings-section-title">Blocked people</h3>
  <p class="hint">Their DMs, calls, invites, pings and sounds don't reach you, and you can't hear them in voice. They see you offline with an empty profile. They're never told. Right-click someone to change how they show in servers you share.</p>
  <button type="button" class="btn btn-secondary add" onclick={() => (picking = true)}><Icon name="plus" size={14} /> Add person</button>
  {#if blocked.length}
    <ul class="settings-list" data-blocked-list>
      {#each blocked as p (p.id)}
        <li class="settings-list-item">
          <button type="button" class="row" aria-haspopup="menu" data-blocked={p.id}
            oncontextmenu={(e) => open(e, p)} onkeydown={(e) => { if (isMenuKey(e)) open(e, p) }}>
            <Avatar id={p.id} name={p.name} size={28} />
            <span class="who">{p.name}</span>
            <span class="mode">{p.mode === 'hide' ? 'Hidden completely' : "Shows a 'Blocked message' line"}</span>
          </button>
        </li>
      {/each}
    </ul>
  {:else}
    <p class="empty">You haven't blocked anyone. Add someone here, or right-click them anywhere and pick Block….</p>
  {/if}
  {#if error}<p class="error" role="alert">{error}</p>{/if}
</div>

{#if menu}
  {@const m = menu}
  <div bind:this={menuEl} class="menu menu-surface pop-in" role="menu" aria-label="Actions for {m.name}" tabindex="-1"
    style:left="{place.left}px" style:top="{place.top}px" style:--pop-origin={place.origin}
    onkeydown={menuKey} use:clickOutside={(e) => close(e.type === 'keydown')}>
    <button type="button" class="menu-item" role="menuitemradio" aria-checked={m.mode === 'line'} data-item="line"
      onclick={() => pick('line')}>Show a line</button>
    <button type="button" class="menu-item" role="menuitemradio" aria-checked={m.mode === 'hide'} data-item="hide"
      onclick={() => pick('hide')}>Hide completely</button>
    <span class="menu-sep" role="separator"></span>
    <button type="button" class="menu-item" role="menuitem" data-item="unblock" onclick={() => pick(null)}>Unblock</button>
  </div>
{/if}

{#if picking}
  <PeoplePicker title="Block" hint="Their DMs, calls, pings and sounds won't reach you. They're never told." only={candidates} max={Infinity}
    action="Block" onDone={add} onClose={() => (picking = false)} />
{/if}

<style>
  p { margin: 0; color: var(--text-2); font-size: 13px; }
  .add { align-self: flex-start; }
  .row { flex: 1; min-width: 0; display: flex; align-items: center; gap: 10px; padding: 0; border: 0; background: none; color: inherit; font: inherit; text-align: left; cursor: default; }
  .row:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 2px; border-radius: var(--radius-sm); }
  .who { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 600; }
  .mode { font-size: 12px; color: var(--text-3); white-space: nowrap; }
  .error { margin: 0; color: var(--danger-text); font-size: 13px; }
  .menu { position: fixed; z-index: 60; width: max-content; min-width: 200px; gap: 2px; outline: none; --pop-origin: top left; }
  .menu-item[aria-checked='true']::after { content: '✓'; margin-left: auto; padding-left: 12px; color: var(--accent-text); }
</style>
