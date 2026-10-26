<script lang="ts">
  import { app } from '../lib/store.svelte'
  import { connLabel } from '../lib/conn'

  // ticks once a second, but only while a "retrying in Ns" countdown is showing
  let now = $state(Date.now())
  const counting = $derived(typeof app.state.conn === 'object')
  $effect(() => {
    if (!counting) return
    now = Date.now()
    const id = setInterval(() => (now = Date.now()), 1000)
    return () => clearInterval(id)
  })
</script>

{connLabel(app.state.conn, app.connAt, now)}
