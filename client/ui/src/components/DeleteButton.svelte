<script lang="ts">
  // The trash button in a message's or quote's action bar. A click asks first ("Delete this
  // message?"), like a clip's Delete; Shift+click deletes straight away, as on Discord.
  import { tick } from 'svelte'
  import Icon from './Icon.svelte'
  import { clickOutside, portal } from '../lib/actions'
  import { placePopover } from '../lib/menuPlace'

  let { what, note = '', onConfirm }: { what: string; note?: string; onConfirm: () => void } = $props()

  let asking = $state(false)
  /** Where the question floats: fixed by the bin, so it never makes the message list longer (on the
   *  last message it would push the chat up). Below the bin, or above it near the window's bottom. It's moved out to
   *  the page (`portal`): left in the action bar's layer, the composer covered it on the last message. */
  let place = $state('')
  const WIDTH = 240
  const ROOM = 120 // about the question's height, until it's on the page to measure
  let box: HTMLDivElement | undefined = $state()
  /** Below the bin, or above it near the window's bottom; never past the window's edges (#454): a long note
   *  scrolls inside the box instead of running off the screen. */
  function placeBy(r: DOMRect) {
    const p = placePopover(r, { width: WIDTH, height: box?.offsetHeight || ROOM }, undefined, 'end')
    const y = p.below ? `top: ${p.top}px` : `bottom: ${p.bottom}px`
    place = `position: fixed; left: ${p.left}px; ${y}; max-height: ${p.maxHeight}px; --pop-origin: ${p.below ? 'top' : 'bottom'} right`
  }
  let keep: HTMLButtonElement | undefined = $state()
  let opener: HTMLButtonElement | undefined = $state()

  async function press(e: MouseEvent) {
    if (e.shiftKey) return onConfirm()
    if (!asking && opener) placeBy(opener.getBoundingClientRect())
    asking = !asking
    await tick()
    // again with its real height (a note makes it taller than the guess)
    if (asking && opener) placeBy(opener.getBoundingClientRect())
    keep?.focus()
  }
  // A scroll would leave it floating away from its message.
  $effect(() => {
    if (!asking) return
    const t = setTimeout(() => document.addEventListener('scroll', closeOnScroll, true))
    return () => {
      clearTimeout(t)
      document.removeEventListener('scroll', closeOnScroll, true)
    }
  })
  const closeOnScroll = () => (asking = false)
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
  <button bind:this={opener} class="trigger" aria-label="Delete" aria-haspopup="dialog" aria-expanded={asking} data-menu-toggle onclick={press}><Icon name="trash" size={16} /></button>
  {#if asking}
    <div bind:this={box} class="ask menu-surface pop-in" role="alertdialog" aria-label="Delete {what}" style={place} use:portal use:clickOutside={close}>
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
  .ask { z-index: 40; width: 240px; max-width: calc(100vw - 16px); overflow-y: auto; padding: 10px; flex-direction: row; flex-wrap: wrap; justify-content: flex-end; gap: 6px; }
  .ask p { flex-basis: 100%; margin: 0 0 4px; font-size: 13px; color: var(--text); }
  .ask small { flex-basis: 100%; margin-top: 2px; font-size: 11px; color: var(--text-3); }
</style>
