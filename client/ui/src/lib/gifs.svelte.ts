// Your favourite GIFs (#58), kept on the server so they follow you across devices. Fetched when the GIF picker
// opens or a GIF in chat first offers its star; a star or unstar shows at once and is undone if the server refuses.
import type { Gif } from './protocol/Gif'
import { api } from './tauri'

export const gifFavs = $state<{ list: Gif[]; loaded: boolean }>({ list: [], loaded: false })

/** Whose favourites `gifFavs` holds: another sign-in fetches its own. */
let owner: string | null = null
let loading: Promise<void> | null = null

/** Fetch `me`'s favourites once (`fresh`: again, as the picker does on opening). A failure leaves what's there. */
export function loadGifFavs(me: string, fresh = false): Promise<void> {
  if (owner !== me) {
    owner = me
    gifFavs.list = []
    gifFavs.loaded = false
    loading = null
  }
  if (loading && !fresh) return loading
  const mine = owner
  loading = api.gifFavourites().then(
    (list) => {
      if (owner !== mine) return
      gifFavs.list = list
      gifFavs.loaded = true
    },
    () => {
      if (owner === mine) loading = null // try again next time
    },
  )
  return loading
}

export const isGifFav = (id: string): boolean => gifFavs.list.some((g) => g.id === id)

/** Star or unstar `gif`: shown at once, put back and rethrown if the server refuses. */
export async function toggleGifFav(gif: Gif): Promise<void> {
  const on = !isGifFav(gif.id)
  const before = gifFavs.list
  gifFavs.list = on ? [gif, ...before] : before.filter((g) => g.id !== gif.id)
  try {
    await api.setGifFavourite(gif, on)
  } catch (e) {
    gifFavs.list = before
    throw e
  }
}

/** Tests: forget everything. */
export function resetGifFavs(): void {
  owner = null
  loading = null
  gifFavs.list = []
  gifFavs.loaded = false
}
