-- #651: the speaking ring others see. NULL is the default (Classic, green).
ALTER TABLE users ADD COLUMN ring_style TEXT;
ALTER TABLE users ADD COLUMN ring_color TEXT;
