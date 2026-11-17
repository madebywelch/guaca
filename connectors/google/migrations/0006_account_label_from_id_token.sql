WITH RECURSIVE
  alphabet (chars) AS (
    SELECT 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_'
  ),
  token (id, rest) AS (
    SELECT id, substr(idToken, instr(idToken, '.') + 1)
    FROM account
    WHERE idToken IS NOT NULL AND instr(idToken, '.') > 0 AND (label IS NULL OR label = '')
  ),
  decoded (id, rest, bytes, valid, hex) AS (
    SELECT
      id,
      substr(rest, 1, CASE instr(rest, '.') WHEN 0 THEN length(rest) ELSE instr(rest, '.') - 1 END),
      length(substr(rest, 1, CASE instr(rest, '.') WHEN 0 THEN length(rest) ELSE instr(rest, '.') - 1 END)) * 3 / 4,
      1,
      ''
    FROM token
    UNION ALL
    SELECT
      id,
      substr(rest, 5),
      bytes,
      valid AND instr(chars, substr(rest, 1, 1)) > 0 AND instr(chars, substr(rest, 2, 1)) > 0 AND instr(chars, substr(rest, 3, 1)) > 0 AND instr(chars, substr(rest, 4, 1)) > 0,
      hex || printf(
        '%06X',
        (instr(chars, substr(rest, 1, 1)) - 1) * 262144 + (instr(chars, substr(rest, 2, 1)) - 1) * 4096 + (instr(chars, substr(rest, 3, 1)) - 1) * 64 + (instr(chars, substr(rest, 4, 1)) - 1)
      )
    FROM decoded, alphabet
    WHERE rest <> ''
  ),
  claims (id, json) AS (
    SELECT id, CAST(unhex(substr(hex, 1, bytes * 2)) AS TEXT)
    FROM decoded
    WHERE rest = '' AND valid AND bytes > 0
  ),
  emails (id, email) AS (
    SELECT id, json_extract(json, '$.email')
    FROM claims
    WHERE json_valid(json) AND json_type(json, '$.email') = 'text' AND json_extract(json, '$.email') <> ''
  )
UPDATE account
SET label = (SELECT email FROM emails WHERE emails.id = account.id)
WHERE id IN (SELECT id FROM emails);

UPDATE account SET idToken = NULL WHERE idToken IS NOT NULL;
