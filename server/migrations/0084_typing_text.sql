-- #652: what shows after "<name> is" while someone types ("cooking", "yapping"). NULL is the plain "is typing".
ALTER TABLE users ADD COLUMN typing_text TEXT;
