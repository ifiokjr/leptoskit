-- dialect: sqlite

ALTER TABLE "notes" ADD COLUMN "pinned" INTEGER NOT NULL DEFAULT 0 CHECK ("pinned" IN (0, 1));
