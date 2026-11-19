-- A server's banner (#475): the hash of a banner upload (media kind 'banner'), set by its owner. NULL: none, and
-- invite cards draw a gradient of its colours.
ALTER TABLE servers ADD COLUMN banner_hash TEXT;
