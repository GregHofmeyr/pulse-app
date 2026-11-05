<script lang="ts">
  // "Loading…" that waits (#173): it shows only once whatever it stands for has taken LOADING_DELAY_MS, counted from
  // when it is rendered, so a quick answer replaces nothing and the text never flashes. Every loading line uses it,
  // so they all wait the same and look the same (app.css .loading).
  import { LOADING_DELAY_MS } from '../lib/loading'

  let { label = 'Loading…' }: { label?: string } = $props()

  let slow = $state(false)
  $effect(() => {
    const timer = setTimeout(() => (slow = true), LOADING_DELAY_MS)
    return () => clearTimeout(timer)
  })
</script>

{#if slow}<p class="loading" role="status">{label}</p>{/if}
