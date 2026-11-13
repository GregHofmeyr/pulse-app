<script lang="ts">
  // A clip's menu: Download for everyone (#204), and Pin clip (a flyout of lengths, as a message's menu has) or Unpin
  // clip (#257); for the poster also Edit (caption and game, in a small dialog) and Delete (asked first). The server
  // refuses anyone else's edit or delete anyway. While the clip downloads (#205) Download reads "Downloading 42%" and
  // Cancel download stops it.
  import { tick } from 'svelte'
  import GamePicker from './GamePicker.svelte'
  import Icon from './Icon.svelte'
  import MenuFlyout from './MenuFlyout.svelte'
  import { clickOutside, modal } from '../lib/actions'
  import { livePin, PIN_FOR } from '../lib/pins'
  import { refreshPins } from '../lib/pinSync.svelte'
  import { serverNow } from '../lib/serverClock'
  import { clock } from '../lib/store.svelte'
  import { api, errorText } from '../lib/tauri'
  import { clipKey, downloads } from '../lib/downloads.svelte'
  import type { Message } from '../lib/protocol/Message'
  import type { PinDuration } from '../lib/protocol/PinDuration'

  let { message, serverId, mine, onDownload, onError = () => {} }: {
    message: Message
    serverId: string
    mine: boolean
    onDownload: () => void
    /** Says why a pin or unpin was refused (the clip's own error line). */
    onError?: (text: string) => void
  } = $props()

  const MAX_CAPTION = 300

  let opener: HTMLButtonElement | undefined = $state()
  let keep: HTMLButtonElement | undefined = $state()
  let menu = $state(false)
  const dl = $derived(downloads.view(clipKey(message.id)))
  const dlText = $derived(
    dl?.phase === 'downloading' ? (dl.fraction === null ? 'Downloading…' : `Downloading ${Math.round(dl.fraction * 100)}%`) : '',
  )
  let asking = $state(false)
  let editing = $state(false)
  let caption = $state('')
  let game = $state<string | null>(null)
  let busy = $state(false)
  let error = $state('')

  function closeMenu(e?: Event) {
    menu = false
    if (e instanceof KeyboardEvent) opener?.focus()
  }

  // --- pins (#257): by the server's clock, as a message's (#258); a pin run out is offered again at once ---
  const pinned = $derived(!!livePin(message, (void clock.nowIso, serverNow())))
  let menuEl = $state<HTMLElement>()
  let flyout = $state<ReturnType<typeof MenuFlyout>>()
  // where the menu sits on screen, for the flyout's placing: under the ⋯ button, its right edge on the button's (its
  // layout box, not its rectangle, which the entrance scales)
  let menuLeft = $state(0)
  let menuTop = $state(0)
  $effect(() => {
    if (!menu || !menuEl || !opener) return
    const r = opener.getBoundingClientRect()
    menuLeft = r.right - menuEl.offsetWidth
    menuTop = r.bottom + 4
  })
  // A refused pin or unpin means the pin here was out of date: say why, and fetch the channel's pins to set it right.
  const report = (p: Promise<unknown>) =>
    p.catch((e) => {
      onError(errorText(e))
      void refreshPins(message.channel_id)
    })
  function pin(d: PinDuration) {
    menu = false
    opener?.focus()
    void report(api.pinMessage(message.id, d))
  }
  function unpin() {
    menu = false
    void report(api.unpinMessage(message.id))
  }
  function download() {
    menu = false
    onDownload()
  }
  function cancelDownload() {
    menu = false
    downloads.cancel(clipKey(message.id))
  }
  function startEdit() {
    menu = false
    caption = message.content
    game = message.clip?.game_id ?? null
    error = ''
    editing = true
  }
  async function askDelete() {
    menu = false
    error = ''
    asking = true
    await tick()
    keep?.focus() // this copy's Cancel (the same clip can also be pinned in its replies panel)
  }
  async function run(f: () => Promise<unknown>, done: () => void) {
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
  // The edit's MessageUpdated (and the delete's MessageDeleted) change the feeds; nothing is applied here.
  const save = () => run(() => api.editClip(message.id, { caption: caption.trim(), game_id: game }), () => (editing = false))
  const remove = () => run(() => api.deleteMessage(message.id), () => (asking = false))
  const cancelEdit = () => {
    if (!busy) editing = false
  }
</script>

<div class="clip-actions">
  <button bind:this={opener} type="button" class="icon-btn icon-btn-sm" aria-label="Clip actions" aria-haspopup="menu" aria-expanded={menu} data-menu-toggle
    onclick={() => (menu = !menu)}><Icon name="dots" size={16} /></button>
  {#if menu}
    <div bind:this={menuEl} class="menu menu-surface pop-in" role="menu" tabindex="-1" use:clickOutside={closeMenu}
      onpointerover={(e) => {
        // another of the menu's own items under the pointer closes the pin flyout
        const t = (e.target as Element).closest?.('.menu-item')
        if (t && t.parentElement === menuEl && t !== flyout?.trigger()) flyout?.closeSoon()
      }}>
      {#if dl?.phase === 'downloading'}
        <span class="menu-item progress" role="menuitem" aria-disabled="true"><Icon name="download" size={14} />{dlText}</span>
        <button type="button" class="menu-item" role="menuitem" onclick={cancelDownload}><Icon name="x" size={14} />Cancel download</button>
      {:else}
        <button type="button" class="menu-item" role="menuitem" onclick={download}><Icon name={dl?.phase === 'saved' ? 'check' : 'download'} size={14} />{dl?.phase === 'saved' ? 'Saved' : 'Download'}</button>
      {/if}
      {#if pinned}
        <button type="button" class="menu-item" role="menuitem" onclick={unpin}><Icon name="pin" size={14} />Unpin clip</button>
      {:else}
        <MenuFlyout bind:this={flyout} label="Pin clip" flyoutLabel="Pin clip for" {menuLeft} {menuTop} item="pin" icon="pin"
          choices={PIN_FOR} onPick={(i) => pin(PIN_FOR[i].duration)} />
      {/if}
      {#if mine}
        <button type="button" class="menu-item" role="menuitem" onclick={startEdit}><Icon name="pencil" size={14} />Edit</button>
        <button type="button" class="menu-item danger" role="menuitem" onclick={() => void askDelete()}><Icon name="trash" size={14} />Delete</button>
      {/if}
    </div>
  {/if}
  {#if asking}
    <div class="ask menu-surface pop-in" role="alertdialog" aria-label="Delete clip" use:clickOutside={() => { if (!busy) asking = false }}>
      <p>Delete this clip? Its replies go too.</p>
      <button bind:this={keep} type="button" class="btn btn-secondary keep" disabled={busy} onclick={() => (asking = false)}>Cancel</button>
      <button type="button" class="btn btn-danger go" disabled={busy} onclick={() => void remove()}>Delete</button>
      {#if error}<p class="err" role="alert">{error}</p>{/if}
    </div>
  {/if}
</div>

{#if editing}
  <div class="overlay modal-overlay fade-in">
    <div class="backdrop modal-backdrop" role="presentation" data-backdrop onclick={cancelEdit}></div>
    <div class="modal pop-in" role="dialog" aria-modal="true" aria-label="Edit clip" tabindex="-1" use:modal={cancelEdit}>
      <h1 class="modal-title">Edit clip</h1>
      <textarea class="input" aria-label="Caption" placeholder="Say something about it (optional)" maxlength={MAX_CAPTION} rows="3" bind:value={caption} disabled={busy}></textarea>
      <GamePicker {serverId} value={game} onChange={(id) => (game = id)} />
      {#if error}<p class="err" role="alert">{error}</p>{/if}
      <div class="foot">
        <button type="button" class="btn btn-lg btn-secondary" disabled={busy} onclick={cancelEdit}>Cancel</button>
        <button type="button" class="btn btn-lg btn-primary" disabled={busy} onclick={() => void save()}>Save</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .clip-actions { position: relative; }
  .menu { position: absolute; z-index: 6; right: 0; top: calc(100% + 4px); min-width: 140px; outline: none; --pop-origin: top right; }
  /* the download's progress: a line of the menu, not a choice */
  .menu-item.progress { color: var(--text-2); cursor: default; font-variant-numeric: tabular-nums; }
  .menu-item.progress:hover { background: none; }
  .ask { position: absolute; z-index: 6; right: 0; top: calc(100% + 4px); width: 240px; padding: 10px; flex-direction: row; flex-wrap: wrap; justify-content: flex-end; gap: 6px; }
  .ask p { flex-basis: 100%; margin: 0 0 4px; font-size: 13px; color: var(--text); }
  .ask .err { margin: 4px 0 0; color: var(--danger-text); }
  .overlay { position: fixed; inset: 0; z-index: 30; }
  .modal { width: min(420px, 100%); }
  textarea { resize: vertical; min-height: 64px; }
  /* a dialog's error line, as in every other dialog */
  .err { margin: 0; font-size: 13px; color: var(--danger-text); }
  .foot { display: flex; justify-content: flex-end; gap: 8px; }
</style>
