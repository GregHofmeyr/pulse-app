# Voice channel chat Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans. TDD each task (failing test first).

**Goal:** every voice channel has a persistent text chat (#226), opened from a chat icon on its row or a toggle on its
stage, shown as a resizable right-side panel that reuses the text channel view.

**Spec:** `docs/superpowers/specs/2026-11-06-voice-chat-design.md`

**Architecture:** the server treats a voice channel as a server chat for messages (a new `ChannelKind::has_chat`,
`is_server_chat` keeps meaning "message-only kinds", which voice's token check relies on). The client shows the chat
in the Shell's right column (it takes the members list's place while open) by mounting `TextChannel` with
`panel` set: a slimmer header (Chat, bell, close), no search box. Open/closed and its own width are kept on this
device.

## Task 1: server, every "voice has no messages" assumption (tests in `server/tests/voice_chat.rs`)

- `protocol`: `ChannelKind::has_chat()` = Text | Quotes | Clips | Voice; test.
- `messages/routes.rs` send: drop the voice refusal; the member check uses `has_chat`.
- `messages/reactions.rs`: the member check uses `has_chat`.
- `access::readable_server_channels` (server search): includes voice channels.
- `servers/routes.rs`: join server and create channel start read points in voice channels too.
- `dms/routes.rs` `set_level`: a voice channel takes a notification level.
- Migration `00NN_voice_chat_reads.sql`: read points for existing members in existing voice channels (at the newest
  message, so nothing old shows as unread). Rehearse on a scratch DB.
- Already generic, covered by tests only: history, edit/delete, mentions, unread counts in Ready, delete → Recently
  deleted (counts vanish), restore, purge removes the messages.
- Ready's `latest` is per DM/group only (conversation list ordering): unchanged, asserted.

## Task 2: client state and unread surface

- `lib/voiceChat.svelte.ts`: `{ open }` persisted (`pulse.voiceChat`), `openVoiceChat()`, `toggleVoiceChat()`, a
  one-shot `focus` flag for the #202 auto-focus on a click.
- `lib/columns`: a third saved width `chat` (default 360), fitted like the right column.
- `ui.openChatId`: the channel whose message list is on screen (a voice channel only while its panel shows), used by
  the alert rule instead of `openChannelId`.

## Task 3: client UI

- `ChannelList` voice row: a chat icon button beside the row (not inside it: no nested buttons) with the unread dot or
  mention count; click opens the stage with the panel.
- `VoicePanel` header: chat toggle (`message` icon, `aria-pressed`) at the top right.
- `Shell`: right column kind `chat` when on a voice stage with the panel open; ColumnHandle; fade-in / exitFade
  (reduced motion and streaming already handled); search jumps and notification clicks into a voice chat open the
  panel.
- `TextChannel` `panel` mode: Chat header with bell and close; focus returns to the stage toggle on close.

## Task 4: verify, review, merge

- server + protocol tests, clippy `-D warnings`; full vitest + svelte-check; xwin clippy only if client Rust changed.
- code review of the branch, fix findings; rebase, rerun, push to main, delete the branch.
