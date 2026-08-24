CREATE SCHEMA "CaseSensitive";
CREATE TABLE "CaseSensitive"."User" ("Id" INTEGER PRIMARY KEY);
WITH RECURSIVE walk(id) AS (
  SELECT "Id" FROM "CaseSensitive"."User"
  UNION ALL
  SELECT id + 1 FROM walk WHERE id < 3
)
SELECT id FROM walk;
