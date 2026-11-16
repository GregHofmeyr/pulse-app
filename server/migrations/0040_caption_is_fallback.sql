-- Attachments follow-up (#297). A message with files and no caption stores the stand-in text (`📎 report.pdf`) as its
-- `content` so older clients have something to show. New clients hide it, and until now the server told them to by
-- comparing `content` with the stand-in on every read, which also hid a caption someone typed that happened to read
-- the same. The flag is now stored when the message is sent or edited.
-- A new column with a constant default (no table rebuild, so foreign keys being OFF while migrating changes nothing).
-- Rollback = leave it (older servers never name it) or `ALTER TABLE messages DROP COLUMN caption_is_fallback`.
ALTER TABLE messages ADD COLUMN caption_is_fallback INTEGER NOT NULL DEFAULT 0 CHECK (caption_is_fallback IN (0, 1));

-- Messages sent before this read as they did: content that equals its files' stand-in is the stand-in. The stand-in
-- is `attachments::fallback_text`: one file's name, or a count of images, or a count of files.
UPDATE messages SET caption_is_fallback = 1
WHERE id IN (SELECT DISTINCT message_id FROM attachments WHERE message_id IS NOT NULL)
  AND content = (
    SELECT CASE
        WHEN COUNT(*) = 1 THEN '📎 ' || MAX(a.name)
        WHEN SUM(a.kind <> 'image') = 0 THEN '📎 ' || COUNT(*) || ' images'
        ELSE '📎 ' || COUNT(*) || ' files'
    END
    FROM attachments a WHERE a.message_id = messages.id
  );
