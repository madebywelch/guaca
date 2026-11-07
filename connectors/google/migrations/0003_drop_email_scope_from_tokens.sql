-- The rewrite `0002` was supposed to do, against the shape the rows are in.
--
-- `0002` narrowed the client row, which is the list the authorize endpoint
-- validates against, so the scope stopped being granted the moment it applied.
-- Its three `UPDATE`s over the token tables matched nothing, and that was worse
-- than doing nothing: the token endpoint re-validates a refresh against the
-- client's allowed scopes, so every machine paired before the change now holds
-- a refresh token carrying an `email` the client no longer allows, and gets
-- `invalid_scope` instead of an hour of access.
--
-- The reason it matched nothing is the encoding. `oauthClient.scopes` was
-- hand-seeded by `0001` and holds a JSON array:
--
--   ["openid","email","offline_access","connectors"]
--
-- The rows Better Auth writes at runtime hold a JSON *string* whose contents
-- are that array's text, which is a different value to `json_each`:
--
--   "[\"openid\",\"email\",\"offline_access\",\"connectors\"]"
--
-- `json_each` on the second one yields a single element, the whole inner text,
-- which is not `email`, so the `WHERE EXISTS` was false for every row. A silent
-- no-op, which is the failure mode a migration should never have: it reported
-- success on 40 rows it had not touched.
--
-- So each row is unwrapped by `json_type`, filtered, and wrapped again the way
-- it was found. `|| ''` in the re-wrap is load-bearing rather than noise:
-- `json_group_array` returns a value carrying SQLite's JSON subtype and
-- `json_quote` passes those through unchanged, so without it the string is
-- never re-quoted and a double-encoded row is silently flattened into a bare
-- array. Concatenation drops the subtype, which is what makes `json_quote`
-- quote.
--
-- Both shapes are handled rather than only the one seen in production, because
-- which one a row has depends on a library's serializer and this file should
-- not have to be right about that a second time. Rows without the scope and
-- rows with no scope at all are left exactly as they are, and running the whole
-- thing twice changes nothing the first pass did not.

UPDATE oauthConsent
SET scopes = (
  SELECT CASE WHEN json_type(oauthConsent.scopes) = 'text'
              THEN json_quote(json_group_array(value) || '')
              ELSE json_group_array(value)
         END
  FROM json_each(
    CASE WHEN json_type(oauthConsent.scopes) = 'text'
         THEN oauthConsent.scopes ->> '$'
         ELSE oauthConsent.scopes END
  )
  WHERE value <> 'email'
)
WHERE scopes IS NOT NULL
  AND EXISTS (
    SELECT 1 FROM json_each(
      CASE WHEN json_type(oauthConsent.scopes) = 'text'
           THEN oauthConsent.scopes ->> '$'
           ELSE oauthConsent.scopes END
    ) WHERE value = 'email'
  );

UPDATE oauthAccessToken
SET scopes = (
  SELECT CASE WHEN json_type(oauthAccessToken.scopes) = 'text'
              THEN json_quote(json_group_array(value) || '')
              ELSE json_group_array(value)
         END
  FROM json_each(
    CASE WHEN json_type(oauthAccessToken.scopes) = 'text'
         THEN oauthAccessToken.scopes ->> '$'
         ELSE oauthAccessToken.scopes END
  )
  WHERE value <> 'email'
)
WHERE scopes IS NOT NULL
  AND EXISTS (
    SELECT 1 FROM json_each(
      CASE WHEN json_type(oauthAccessToken.scopes) = 'text'
           THEN oauthAccessToken.scopes ->> '$'
           ELSE oauthAccessToken.scopes END
    ) WHERE value = 'email'
  );

UPDATE oauthRefreshToken
SET scopes = (
  SELECT CASE WHEN json_type(oauthRefreshToken.scopes) = 'text'
              THEN json_quote(json_group_array(value) || '')
              ELSE json_group_array(value)
         END
  FROM json_each(
    CASE WHEN json_type(oauthRefreshToken.scopes) = 'text'
         THEN oauthRefreshToken.scopes ->> '$'
         ELSE oauthRefreshToken.scopes END
  )
  WHERE value <> 'email'
)
WHERE scopes IS NOT NULL
  AND EXISTS (
    SELECT 1 FROM json_each(
      CASE WHEN json_type(oauthRefreshToken.scopes) = 'text'
           THEN oauthRefreshToken.scopes ->> '$'
           ELSE oauthRefreshToken.scopes END
    ) WHERE value = 'email'
  );
