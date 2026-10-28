-- dialect: sqlite
-- leptoskit: foreign-keys-off

CREATE TABLE "_leptoskit_rebuild_notes" (
	"id" BLOB NOT NULL CHECK (length("id") = 16),
	"body" TEXT NOT NULL,
	"pinned" INTEGER NOT NULL DEFAULT 0 CHECK ("pinned" IN (0, 1)),
	CONSTRAINT "notes_pkey" PRIMARY KEY ("id"),
	CONSTRAINT "notes_body_length" CHECK (length("body") >= 1)
) STRICT;

INSERT INTO "_leptoskit_rebuild_notes" ("id", "body", "pinned") SELECT "id", "body", "pinned" FROM "notes";

DROP TABLE "notes";

ALTER TABLE "_leptoskit_rebuild_notes" RENAME TO "notes";
