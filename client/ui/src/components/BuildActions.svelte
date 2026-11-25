<script lang="ts">
  // A build page's Actions menu (#618): Edit for its author, Archive (or Restore) and Delete for its author or the
  // server's owner (spec §5.1; the server refuses anyone else anyway). Delete asks first. `items` adds the quick-access
  // entries (Pin to side #619, Pop out #620) above them; with nothing to offer the menu isn't drawn at all.
  import { tick } from 'svelte'
  import Icon from './Icon.svelte'
  import { clickOutside } from '../lib/actions'
  import { isArchived, type BuildActionItem } from '../lib/builds'
  import { api, errorText } from '../lib/tauri'
  import type { Message } from '../lib/protocol/Message'

  let { message, canEdit, canArchive, canDelete, items, onEdit = () => {}, onItem = () => {}, onDeleted = () => {} }: {
    message: Message
    canEdit: boolean
    canArchive: boolean
    canDelete: boolean
    /** Quick-access entries, in this order (none in phase 3). */
    items: BuildActionItem[]
    onEdit?: () => void
    onItem?: (item: BuildActionItem) => void
    /** The delete went through (its MessageDeleted also arrives). */
    onDeleted?: () => void
  } = $props()

  const ITEM: Record<BuildActionItem, { label: string; icon: 'pin' | 'expand' }> = {
    pin: { label: 'Pin to side', icon: 'pin' },
    popout: { label: 'Pop out', icon: 'expand' },
  }

  let opener: HTMLButtonElement | undefined = $state()
  let keep: HTMLButtonElement | undefined = $state()
  let menu = $state(false)
  let asking = $state(false)
  let busy = $state(false)
  let error = $state('')
  const archived = $derived(isArchived(message))
  const any = $derived(items.length > 0 || canEdit || canArchive || canDelete)

  function closeMenu(e?: Event) {
    menu = false
    if (e instanceof KeyboardEvent) opener?.focus()
  }
  function pick(f: () => void) {
    menu = false
    f()
  }
  async function run(f: () => Promise<unknown>, done: () => void = () => {}) {
    if (busy) return
    busy = true
    error = ''
    try {
      await f()
      done()
    } catch (e) {
      error = errorText(e)
    } finally {
      busy = false
    }
  }
  // The MessageUpdated (or BuildsArchived) that follows changes the copies everywhere; nothing is applied here.
  const archive = () => pick(() => void run(() => api.setBuildArchived(message.id, !archived)))
  async function askDelete() {
    menu = false
    error = ''
    asking = true
    await tick()
    keep?.focus()
  }
  function askKey(e: KeyboardEvent) {
    if (e.key !== 'Escape' || busy) return
    e.preventDefault()
    e.stopPropagation()
    asking = false
    opener?.focus()
  }
  const remove = () => run(() => api.deleteBuild(message.id), () => {
    asking = false
    onDeleted()
  })
</script>

{#if any}
  <div class="build-actions">
    <button bind:this={opener} type="button" class="btn btn-secondary" aria-haspopup="menu" aria-expanded={menu} data-menu-toggle
      onclick={() => (menu = !menu)}>Actions</button>
    {#if menu}
      <div class="menu menu-surface pop-in" role="menu" tabindex="-1" use:clickOutside={closeMenu}
        onkeydown={(e) => { if (e.key === 'Escape') { e.stopPropagation(); closeMenu(e) } }}>
        {#each items as it (it)}
          <button type="button" class="menu-item" role="menuitem" onclick={() => pick(() => onItem(it))}><Icon name={ITEM[it].icon} size={14} />{ITEM[it].label}</button>
        {/each}
        {#if canEdit}
          <button type="button" class="menu-item" role="menuitem" onclick={() => pick(onEdit)}><Icon name="pencil" size={14} />Edit</button>
        {/if}
        {#if canArchive}
          <button type="button" class="menu-item" role="menuitem" onclick={archive}><Icon name="inbox" size={14} />{archived ? 'Restore' : 'Archive'}</button>
        {/if}
        {#if canDelete}
          <button type="button" class="menu-item danger" role="menuitem" onclick={() => void askDelete()}><Icon name="trash" size={14} />Delete</button>
        {/if}
      </div>
    {/if}
    {#if asking}
      <div class="ask menu-surface pop-in" role="alertdialog" aria-label="Delete build" tabindex="-1"
        use:clickOutside={() => { if (!busy) asking = false }} onkeydown={askKey}>
        <p>Delete this build? Its screenshots and comments go too.</p>
        <button bind:this={keep} type="button" class="btn btn-secondary" disabled={busy} onclick={() => (asking = false)}>Cancel</button>
        <button type="button" class="btn btn-danger" disabled={busy} onclick={() => void remove()}>Delete</button>
      </div>
    {/if}
    {#if error}<p class="err" role="alert">{error}</p>{/if}
  </div>
{/if}

<style>
  .build-actions { position: relative; display: flex; flex-direction: column; align-items: flex-end; }
  .menu { position: absolute; z-index: 6; right: 0; top: calc(100% + 4px); min-width: 160px; outline: none; --pop-origin: top right; }
  .ask { position: absolute; z-index: 6; right: 0; top: calc(100% + 4px); width: 260px; padding: 10px; flex-direction: row; flex-wrap: wrap; justify-content: flex-end; gap: 6px; }
  .ask p { flex-basis: 100%; margin: 0 0 4px; font-size: 13px; color: var(--text); }
  .err { margin: 4px 0 0; color: var(--danger-text); font-size: 12px; max-width: 260px; text-align: right; }
</style>
