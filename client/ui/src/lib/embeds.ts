// Pure helpers for link preview cards. Everything here works from the server's structured
// `LinkEmbed`; player URLs only ever point at our own embed page (`<origin>/embed/<provider>/<id>`),
// which picks the provider's official player from a validated id.
import type { EmbedProvider } from './protocol/EmbedProvider'
import type { LinkEmbed } from './protocol/LinkEmbed'

type PlayerProvider = Exclude<EmbedProvider, 'generic' | 'unknown'>

const NAMES: Record<PlayerProvider, string> = {
  youtube: 'YouTube',
  tiktok: 'TikTok',
  spotify: 'Spotify',
  soundcloud: 'SoundCloud',
  twitch: 'Twitch',
  instagram: 'Instagram',
}

const isPlayerProvider = (p: EmbedProvider): p is PlayerProvider => Object.hasOwn(NAMES, p)

function host(url: string): string {
  try {
    const h = new URL(url).hostname
    return h.startsWith('www.') ? h.slice(4) : h
  } catch {
    return url
  }
}

/** The card's label: the provider's name ("Instagram · reel"/"· post"), else the site's name or host. */
export function providerLabel(e: LinkEmbed): string {
  if (e.provider === 'instagram') {
    const kind = e.provider_id?.split(':')[0]
    return kind === 'reel' ? 'Instagram · reel' : kind === 'p' ? 'Instagram · post' : 'Instagram'
  }
  if (isPlayerProvider(e.provider)) return NAMES[e.provider]
  return e.site_name?.trim() || host(e.url)
}

/** A player this build knows, with its id: gets a play button. */
export function playable(e: LinkEmbed): boolean {
  return e.kind === 'player' && isPlayerProvider(e.provider) && !!e.provider_id
}

/** Instagram plays in a capability-free pop-out window, not inline. */
export const popsOut = (e: LinkEmbed): boolean => e.provider === 'instagram'

/** Our embed page for this player (YouTube keeps its start time). */
export function playerSrc(origin: string, e: LinkEmbed): string {
  const base = `${origin.replace(/\/+$/, '')}/embed/${e.provider}/${encodeURIComponent(e.provider_id ?? '')}`
  const start = e.provider === 'youtube' && e.start && e.start > 0 ? Math.floor(e.start) : 0
  return start ? `${base}?start=${start}` : base
}

/** The player's shape: 16:9, 9:16 (TikTok, Shorts, reels) or a fixed-height audio strip. */
export function aspect(e: LinkEmbed): '16 / 9' | '9 / 16' | 'audio' {
  if (e.provider === 'spotify' || e.provider === 'soundcloud') return 'audio'
  return e.vertical ? '9 / 16' : '16 / 9'
}

/** An audio player's height in px (the providers' compact players). */
export const audioHeight = (e: LinkEmbed): number => (e.provider === 'soundcloud' ? 166 : 152)

const PLAYER_SANDBOX = 'allow-scripts allow-same-origin allow-presentation'
/** Each player's iframe sandbox: what its official player needs and no popups. */
export const SANDBOX: Record<PlayerProvider, string> = {
  youtube: PLAYER_SANDBOX,
  tiktok: PLAYER_SANDBOX,
  spotify: PLAYER_SANDBOX,
  soundcloud: PLAYER_SANDBOX,
  twitch: PLAYER_SANDBOX,
  instagram: PLAYER_SANDBOX,
}

export const sandboxFor = (e: LinkEmbed): string => (isPlayerProvider(e.provider) ? SANDBOX[e.provider] : PLAYER_SANDBOX)
