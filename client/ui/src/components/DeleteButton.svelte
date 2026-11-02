<script lang="ts">
  // The trash button in a message's or quote's action bar. A click asks first ("Delete this
  // message?"), like a clip's Delete; Shift+click deletes straight away, as on Discord.
  import { tick } from 'svelte'
  import Icon from './Icon.svelte'
  import { clickOutside } from '../lib/actions'

  let { what, note = '', onConfirm }: { what: string; note?: string; onConfirm: () => void } = $props()

  let asking = $state(false)
  let keep: HTMLButtonElement | undefined = $state()
  let opener: HTMLButtonElement | undefined = $state()

  async function press(e: MouseEvent) {
    if (e.shiftKey) return onConfirm()
    asking = !asking
    await tick()
    keep?.focus()
  }
  function close(e?: Event) {
    asking = false
    if (e instanceof KeyboardEvent) opener?.focus()
  }
  function confirm() {
    asking = false
    onConfirm()
  }
</script>

<span class="del">
  <button bind:this={opener} class="trigger" aria-label="Delete" aria-haspopup="dialog" aria-expanded={asking} data-menu-toggle onclick={press}><Icon name="trash" size={15} /></button>
  {#if asking}
    <div class="ask menu-surface pop-in" role="alertdialog" aria-label="Delete {what}" use:clickOutside={close}>
      <p>Delete this {what}?{note ? ` ${note}` : ''}</p>
      <button bind:this={keep} type="button" class="btn btn-secondary" onclick={() => close()}>Cancel</button>
      <button type="button" class="btn btn-danger go" onclick={confirm}>Delete</button>
      <small>Shift+click Delete to skip this.</small>
    </div>
  {/if}
</span>

<style>
  .del { position: relative; display: flex; }
  /* the action bar's look (MessageItem / QuoteItem `.actions button`) */
  .trigger { width: 30px; height: 28px; border: 0; background: transparent; color: var(--text-2); display: grid; place-items: center; }
  .trigger:hover { color: var(--danger-text); }
  .ask { position: absolute; z-index: 6; right: 0; top: calc(100% + 6px); width: 240px; padding: 10px; flex-direction: row; flex-wrap: wrap; justify-content: flex-end; gap: 6px; }
  .ask p { flex-basis: 100%; margin: 0 0 4px; font-size: 13px; color: var(--text); }
  .ask small { flex-basis: 100%; margin-top: 2px; font-size: 11px; color: var(--text-3); }
</style>
