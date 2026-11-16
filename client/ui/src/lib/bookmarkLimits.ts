// The server's saved message limits (#315, protocol `bookmark_limits`), so the app can say them before it refuses.
export const bookmark_limits = {
  MAX_BOOKMARKS: 500,
  NOTE_MAX_CHARS: 500,
  SNIPPET_CHARS: 300,
} as const
