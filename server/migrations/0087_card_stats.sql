-- #727: which stats a card shows, in what order: a JSON array of keys. NULL is the default (all four).
ALTER TABLE users ADD COLUMN card_stats TEXT;
