-- The desktop app stops being told the account's email address.
--
-- `email` was in the desktop client's scope list from the day the client was
-- seeded, so `/oauth2/userinfo` returned an `email` claim and `/api/connectors`
-- passed it on as `user.email`. That was a mistake rather than a feature. The
-- address a sign-in code reached is this origin's name for an account; it is
-- not something a paired machine needs, and the app already has the address
-- that matters to it — the Google or GitHub identity on each connection, which
-- comes from the provider's own id token and says which mailbox a tool acts on.
-- One scope kept for one UI string put a personal detail on every laptop
-- running Guaca and in every backup of one.
--
-- Removing it from `SCOPES` in `worker/auth.ts` is not enough on its own. The
-- authorization endpoint validates against `client.scopes ?? opts.scopes`, so
-- the row below is the list that decides; `worker.itest.ts` compares the two so
-- they cannot drift apart again.
--
-- The three token tables are rewritten rather than left alone, for two separate
-- reasons. A live access token carries its own granted scopes and would keep
-- yielding the claim until it expired. A refresh token is worse than stale: the
-- token endpoint re-validates a refresh against the client's allowed scopes, so
-- a stored `email` the client no longer allows would fail every refresh with
-- `invalid_scope` and sign out every already-paired machine. Narrowing what is
-- stored keeps those machines linked and simply stops answering the claim.
--
-- Those three `UPDATE`s do not work, and this file is left as it is because it
-- has already been applied. They assume the rows hold a JSON array, and the
-- rows Better Auth writes hold a JSON *string* containing that array's text, so
-- `json_each` saw one opaque element and the `WHERE EXISTS` was false every
-- time. They matched nothing and reported success.
-- `0003_drop_email_scope_from_tokens.sql` is the rewrite that works, and it is
-- required: without it the `UPDATE` below has narrowed the client while every
-- machine paired before it still holds a refresh token the client now refuses.
--
-- What this does break is a copy of Guaca old enough to still ask for `email`
-- in a *new* authorization request: it gets `invalid_scope` on the loopback
-- redirect. That is the honest failure. The alternative is accepting a scope
-- whose consent line is a promise this origin no longer keeps.

UPDATE oauthClient
SET scopes = '["openid","offline_access","connectors"]',
    updatedAt = unixepoch()
WHERE clientId = 'guaca-desktop';

UPDATE oauthConsent
SET scopes = (
  SELECT json_group_array(value) FROM json_each(oauthConsent.scopes) WHERE value <> 'email'
)
WHERE EXISTS (SELECT 1 FROM json_each(oauthConsent.scopes) WHERE value = 'email');

UPDATE oauthAccessToken
SET scopes = (
  SELECT json_group_array(value) FROM json_each(oauthAccessToken.scopes) WHERE value <> 'email'
)
WHERE EXISTS (SELECT 1 FROM json_each(oauthAccessToken.scopes) WHERE value = 'email');

UPDATE oauthRefreshToken
SET scopes = (
  SELECT json_group_array(value) FROM json_each(oauthRefreshToken.scopes) WHERE value <> 'email'
)
WHERE EXISTS (SELECT 1 FROM json_each(oauthRefreshToken.scopes) WHERE value = 'email');
