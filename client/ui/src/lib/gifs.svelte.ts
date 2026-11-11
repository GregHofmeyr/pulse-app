// Your favourite GIFs (#58), kept on the server so they follow you across devices. Fetched when the GIF picker
// opens or a GIF in chat first offers its star; a star or unstar shows at once and is undone if the server refuses.
// A fetch that started before a star or unstar is dropped (it would undo it on screen), and one GIF's changes go to
// the server one at a time, in order (a quick star and unstar can't land the other way round).
import type { Gif } from './protocol/Gif'
import { api } from './tauri'

export const gifFavs = $state<{ list: Gif[]; loaded: boolean }>({ list: [], loaded: false })

/** Whose favourites `gifFavs` holds: another sign-in fetches its own. */
let owner: string | null = null
let loading: Promise<void> | null = null
/** Bumped by every star and unstar: a fetch begun before the latest one is out of date. */
let changes = 0
/** Each GIF's last change on its way to the server. */
const sending = new Map<string, Promise<unknown>>()

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
  const at = changes
  const p: Promise<void> = api.gifFavourites().then(
    (list) => {
      // a newer fetch or another sign-in: this answer is moot
      if (owner !== mine || loading !== p) return
      // a star since: this answer predates it, so ask again (once the change has gone out)
      if (changes !== at) return Promise.all([...sending.values()].map((s) => s.catch(() => {}))).then(() => loadGifFavs(mine!, true))
      gifFavs.list = list
      gifFavs.loaded = true
    },
    () => {
      if (owner === mine && loading === p) loading = null // try again next time
    },
  )
  loading = p
  return p
}

export const isGifFav = (id: string): boolean => gifFavs.list.some((g) => g.id === id)

/** Star or unstar `gif`: shown at once; if the server refuses, that GIF alone goes back and the error is rethrown. */
export async function toggleGifFav(gif: Gif): Promise<void> {
  const on = !isGifFav(gif.id)
  changes++
  gifFavs.list = on ? [gif, ...gifFavs.list] : gifFavs.list.filter((g) => g.id !== gif.id)
  const send = (sending.get(gif.id) ?? Promise.resolve())
    .catch(() => {})
    .then(() => api.setGifFavourite(gif, on))
  sending.set(gif.id, send)
  try {
    await send
  } catch (e) {
    // only if nothing newer changed this GIF since
    if (sending.get(gif.id) === send) {
      gifFavs.list = on ? gifFavs.list.filter((g) => g.id !== gif.id) : isGifFav(gif.id) ? gifFavs.list : [gif, ...gifFavs.list]
    }
    throw e
  } finally {
    if (sending.get(gif.id) === send) sending.delete(gif.id)
  }
}

/** Tests: forget everything. */
export function resetGifFavs(): void {
  owner = null
  loading = null
  changes = 0
  sending.clear()
  gifFavs.list = []
  gifFavs.loaded = false
}
