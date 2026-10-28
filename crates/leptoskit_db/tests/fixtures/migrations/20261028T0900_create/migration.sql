-- dialect: sqlite

CREATE TABLE "notes" (
	"id" BLOB NOT NULL CHECK (length("id") = 16),
	"body" TEXT NOT NULL,
	CONSTRAINT "notes_pkey" PRIMARY KEY ("id")
) STRICT;

CREATE TABLE "labels" (
	"id" BLOB NOT NULL CHECK (length("id") = 16),
	"note_id" BLOB NOT NULL CHECK (length("note_id") = 16),
	"text" TEXT NOT NULL,
	CONSTRAINT "labels_pkey" PRIMARY KEY ("id"),
	CONSTRAINT "labels_note_id_fkey" FOREIGN KEY ("note_id") REFERENCES "notes" ("id") ON DELETE RESTRICT
) STRICT;
