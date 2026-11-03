<script lang="ts">
  // A password field with an eye inside it that shows what's typed (#131). Every password field uses it,
  // with its own aria-label: the eye sits inside the caller's <label>, whose text would take in "Show password".
  import type { HTMLInputAttributes } from 'svelte/elements'
  import Icon from './Icon.svelte'

  let { value = $bindable(''), ...rest }: Omit<HTMLInputAttributes, 'type' | 'value'> & { value?: string } = $props()

  let shown = $state(false)
  let input = $state<HTMLInputElement>()

  // a sent form never leaves the password showing
  $effect(() => {
    const form = input?.form
    if (!form) return
    const hide = () => { shown = false }
    form.addEventListener('submit', hide)
    return () => form.removeEventListener('submit', hide)
  })

  function toggle() {
    const el = input!
    const had = document.activeElement === el
    const start = el.selectionStart
    const end = el.selectionEnd
    shown = !shown
    // switching the type can reset the selection: put the caret back where it was
    el.type = shown ? 'text' : 'password'
    if (had) {
      el.focus()
      if (start !== null && end !== null) el.setSelectionRange(start, end)
    }
  }
</script>

<span class="field">
  <input {...rest} bind:this={input} type={shown ? 'text' : 'password'} bind:value />
  <button type="button" class="eye" aria-label={shown ? 'Hide password' : 'Show password'} aria-pressed={shown}
    onmousedown={(e) => e.preventDefault()} onclick={toggle}>
    <Icon name={shown ? 'eyeOff' : 'eye'} size={16} />
  </button>
</span>

<style>
  .field { position: relative; display: flex; }
  input { flex: 1; min-width: 0; padding-right: 36px; }
  .eye {
    position: absolute; top: 0; right: 2px; bottom: 0; margin: auto 0; width: 30px; height: 28px;
    display: grid; place-items: center; padding: 0; border: 0; border-radius: 6px;
    background: transparent; color: var(--text-3); cursor: pointer;
  }
  .eye:hover { color: var(--text); }
  .eye:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: -2px; }
</style>
