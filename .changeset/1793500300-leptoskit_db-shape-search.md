---
"release": minor
---

# Search results as shapes

A full-text search can return each match as a shape: `Article::query().search("passkeys").shape::<ArticleCard>().hits(&db)` gets each article with its linked rows in one query, still ranked best first, filtered, limited, and highlighted. Nested rows follow their own tables' policies, as in any shape.
