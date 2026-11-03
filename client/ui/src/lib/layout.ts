// Side panels in a narrow window (#97): the window body (sidebar + chat + panels) below this many pixels counts as
// narrow, and only one side panel shows there.
export const NARROW_BODY = 1100

/** Whether the search results step aside: narrow, searching, and a replies panel (a quote's or a clip's) is open.
 *  They're hidden, not closed, so closing the replies panel brings them back as they were. */
export function searchMakesRoom(s: { narrow: boolean; searchOpen: boolean; threadOpen: boolean }): boolean {
  return s.narrow && s.searchOpen && s.threadOpen
}
