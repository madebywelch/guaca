-- The desktop app is told which Guaca account it is paired to, again.
--
-- `0002` narrowed the client to `["openid","offline_access","connectors"]` on
-- the reasoning that the address a sign-in code reached is this origin's name
-- for an account and nothing the app needs. The second half of that is wrong.
-- An operator can hold two accounts, and a machine linked to the one that does
-- not hold the grant they are reading in a browser looks exactly like a broken
-- connector: Settings draws no identity, the agents find no tools, and nothing
-- on either side names the account to tell the two apart.
--
-- It also broke every copy of Guaca rather than only an old one. `0002` said
-- the cost was "a copy old enough to still ask for `email` in a new
-- authorization request", but no copy has ever asked for anything else:
-- `SCOPE` in the app's `src-tauri/src/account.rs` is
-- `openid email offline_access connectors`, so `oauthProvider` refused every
-- pairing with `invalid_scope` against `client.scopes`, and the app's own
-- `/api/connectors` reader requires the `user` object that went with it.
--
-- What was right about `0002` is the boundary, and that is kept: the address
-- goes to the desktop app and no further. The MCP server takes a `Caller` with
-- no field for it, so no tool list, tool result or prompt carries it. See
-- `SCOPES` in `worker/auth.ts`.
--
-- The client row only. `0003` stripped the scope from the token tables, and
-- those rows stay as they are: a stored grant narrower than what the client
-- allows is valid, so a machine paired in between keeps refreshing and simply
-- gets no claim until it authorizes again. Widening tokens already issued would
-- be this migration granting a scope nobody asked for.

UPDATE oauthClient
SET scopes = '["openid","email","offline_access","connectors"]',
    updatedAt = unixepoch()
WHERE clientId = 'guaca-desktop';
