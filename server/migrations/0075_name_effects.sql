-- Name effects (#468): one of the effect keys (profiles::NAME_EFFECTS), or NULL for none. Lives on
-- `users` next to name_font and name_color.
ALTER TABLE users ADD COLUMN name_effect TEXT;
