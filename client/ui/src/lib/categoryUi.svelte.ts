// "Create category" (#54) is chosen from menus (the server tab's, the sidebar header's) but typed into the channel
// list: the menu posts a request here and the list opens its inline field. `seq` makes each request new, so asking
// twice for the same server opens the field again.
export const categoryUi = $state({ create: null as { serverId: string; seq: number } | null })

let seq = 0

export function requestCreateCategory(serverId: string): void {
  categoryUi.create = { serverId, seq: ++seq }
}
