<script lang="ts">
  // The download icon on a clip's video (#204) or a quoted image (#206), Discord-style: top right, shown while the
  // pointer is over it (the parent's :hover rule) or when it has keyboard focus. `beside`: left of the wall card's
  // full-screen button.
  import Icon from './Icon.svelte'

  let { onclick, beside = false, label = 'Download clip', tip = 'Download' }: {
    onclick: (e: MouseEvent) => void
    beside?: boolean
    label?: string
    tip?: string
  } = $props()
</script>

<button type="button" class="clip-dl" class:beside aria-label={label} data-tip={tip} {onclick}><Icon name="download" size={16} /></button>

<style>
  /* the wall card's full-screen button's look (ClipCard .fs) */
  .clip-dl { position: absolute; z-index: 1; right: 8px; top: 8px; width: 28px; height: 28px; border: 0; border-radius: var(--radius-sm); background: rgba(0, 0, 0, .6); color: #fff; display: grid; place-items: center; cursor: pointer; opacity: 0; transition: opacity var(--dur) var(--ease), background-color var(--dur) var(--ease); }
  .clip-dl:hover { background: rgba(0, 0, 0, .75); }
  .clip-dl:focus-visible { opacity: 1; }
  .clip-dl.beside { right: 40px; }
</style>
