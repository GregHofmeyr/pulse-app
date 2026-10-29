<script lang="ts">
  import { app } from '../lib/store.svelte'
  import { appearance } from '../lib/appearance.svelte'
  import { displayName, roleColorFor, userOf } from '../lib/selectors'
  import { nameCss, resolveNameStyle } from '../lib/nameStyle'
  import { openCard } from '../lib/profileCard.svelte'

  import type { User } from '../lib/protocol/User'

  let { userId, serverId, text, interactive = true, size = 'normal', user }: {
    userId: string
    serverId: string | null
    text?: string
    interactive?: boolean
    size?: 'normal' | 'card'
    /** Style the name as this user instead of the stored one (the profile editor's preview). */
    user?: User
  } = $props()

  const css = $derived(nameCss(resolveNameStyle(user ?? userOf(app.state, userId), { roleColor: roleColorFor(app.state, serverId, userId), showStyles: appearance.showNameStyles })))
  const label = $derived(text ?? displayName(app.state, serverId, userId))
</script>

{#if interactive}
  <button type="button" class="name" class:card={size === 'card'} style={css} onclick={(e) => openCard(userId, serverId, e.currentTarget)}>{label}</button>
{:else}
  <span class="name" class:card={size === 'card'} style={css}>{label}</span>
{/if}

<style>
  /* The font scale evens out the fonts; the line height stays at the parent's, so rows keep their height. */
  .name { display: inline-block; font-size: calc(1em * var(--name-scale, 1)); line-height: calc(1.4em / var(--name-scale, 1)); vertical-align: bottom; max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  /* line height in the card's own em: still the parent's 1.4 */
  .name.card { font-size: calc(1.15em * var(--name-scale, 1)); line-height: calc(1.4em / (1.15 * var(--name-scale, 1))); }
  button.name { background: none; border: 0; padding: 0; margin: 0; font-family: inherit; font-weight: inherit; color: inherit; cursor: pointer; text-align: left; }
  button.name:hover { text-decoration: underline; }
</style>
