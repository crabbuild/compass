CREATE SCHEMA "accounting";

CREATE TABLE "accounting"."accounts" (
    "id" BIGINT PRIMARY KEY,
    "owner_id" BIGINT NOT NULL,
    CONSTRAINT "accounts_owner_fk"
        FOREIGN KEY ("owner_id") REFERENCES "accounting"."users"("id")
);

CREATE TABLE "accounting"."users" (
    "id" BIGINT PRIMARY KEY
);

CREATE UNIQUE INDEX "accounts_owner_idx"
    ON "accounting"."accounts"("owner_id");

CREATE VIEW "accounting"."account_owners" AS
    SELECT account.id
    FROM "accounting"."accounts" account
    JOIN "accounting"."users" owner ON owner.id = account.owner_id;

CREATE PROCEDURE "accounting"."refresh_accounts"()
LANGUAGE SQL
AS $$
    UPDATE "accounting"."accounts" SET "owner_id" = "owner_id";
$$;

CREATE TRIGGER "accounts_audit"
AFTER UPDATE ON "accounting"."accounts"
FOR EACH ROW EXECUTE FUNCTION audit_account();

INSERT INTO "accounting"."accounts"("id", "owner_id")
SELECT "id", "id" FROM "accounting"."users";
