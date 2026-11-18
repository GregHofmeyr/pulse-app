-- Watch party queue (#464): a queued video takes its title and thumbnail from a message's link preview of it, found
-- by provider and id.
CREATE INDEX link_embeds_provider ON link_embeds(provider, provider_id) WHERE provider_id IS NOT NULL;
