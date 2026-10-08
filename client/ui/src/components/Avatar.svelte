<script lang="ts">
  import { avatarColor, initial } from '../lib/avatar'
  let { id, name, size = 30, online = null }: { id: string; name: string; size?: number; online?: boolean | null } = $props()
</script>

<span class="av" style:width="{size}px" style:height="{size}px" style:font-size="{Math.round(size * 0.42)}px" style:background={avatarColor(id)}>
  {initial(name)}
  {#if online !== null}<span class="pres" class:off={!online} role="img" aria-label={online ? 'online' : 'offline'}></span>{/if}
</span>

<style>
  .av { position: relative; flex-shrink: 0; border-radius: 50%; display: grid; place-items: center; font-weight: 700; color: #fff; }
  .pres { position: absolute; right: -2px; bottom: -2px; width: 34%; height: 34%; min-width: 9px; min-height: 9px; border-radius: 50%; background: var(--ok); border: 2px solid var(--bg-1); box-sizing: border-box; }
  .pres.off { background: #5b5f6b; }
</style>
