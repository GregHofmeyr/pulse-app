-- #547: an invite stops working when its maker leaves or is removed from its server (#474 revokes them then), but ones
-- made before that rule whose maker had already gone were left live: refused on accept, yet still shown as valid by the
-- preview, the invite page and the owner's Invites list. Revoke them once, now; a leave keeps it so from here.
-- Rollback = nothing to undo: an older server refuses these on accept all the same.
UPDATE server_invites SET revoked_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
WHERE revoked_at IS NULL
  AND NOT EXISTS (SELECT 1 FROM server_members m
                  WHERE m.server_id = server_invites.server_id AND m.user_id = server_invites.created_by);
